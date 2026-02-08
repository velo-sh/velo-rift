use anyhow::Result;
use clap::{Parser, Subcommand};

use tokio::signal;

#[derive(Parser)]
#[command(name = "vriftd")]
#[command(version, about = "Velo Rift Daemon", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Start the daemon (default)
    Start,
}

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize tracing
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_env("VRIFT_LOG")
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let cli = Cli::parse();

    match cli.command.unwrap_or(Commands::Start) {
        Commands::Start => start_daemon().await?,
    }

    Ok(())
}

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use tokio::net::{UnixListener, UnixStream};
use vrift_config::path::is_within_directory;
use vrift_ipc::{VeloError, VeloErrorKind, VeloRequest, VeloResponse};
use vrift_manifest::lmdb::{AssetTier, LmdbManifest};

// RFC-0043: Minimal registry for workspace discovery
// TEMPORARILY DISABLED: Investigating UE blocking issues
#[allow(dead_code)]
#[derive(serde::Deserialize)]
struct MinimalManifestEntry {
    project_root: PathBuf,
}

#[allow(dead_code)]
#[derive(serde::Deserialize)]
struct MinimalRegistry {
    manifests: std::collections::HashMap<String, MinimalManifestEntry>,
}

#[allow(dead_code)]
fn load_registered_workspaces() -> Vec<PathBuf> {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/root".to_string());
    let path = PathBuf::from(home).join(".vrift/registry/manifests.json");
    if !path.exists() {
        return Vec::new();
    }

    if let Ok(file) = std::fs::File::open(path) {
        if let Ok(registry) = serde_json::from_reader::<_, MinimalRegistry>(file) {
            return registry
                .manifests
                .values()
                .map(|e| e.project_root.clone())
                .collect();
        }
    }
    Vec::new()
}

#[derive(Debug, Clone, Copy)]
struct PeerCredentials {
    uid: u32,
    #[allow(dead_code)]
    gid: u32,
    pid: Option<libc::pid_t>,
}

impl PeerCredentials {
    #[cfg(target_os = "linux")]
    fn from_stream(stream: &UnixStream) -> Option<Self> {
        use std::os::unix::io::AsRawFd;
        let fd = stream.as_raw_fd();
        let mut cred: libc::ucred = unsafe { std::mem::zeroed() };
        let mut len = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
        let ret = unsafe {
            libc::getsockopt(
                fd,
                libc::SOL_SOCKET,
                libc::SO_PEERCRED,
                &mut cred as *mut _ as *mut libc::c_void,
                &mut len,
            )
        };
        if ret == 0 {
            Some(Self {
                uid: cred.uid,
                gid: cred.gid,
                pid: Some(cred.pid),
            })
        } else {
            None
        }
    }

    #[cfg(target_os = "macos")]
    fn from_stream(stream: &UnixStream) -> Option<Self> {
        use std::os::unix::io::AsRawFd;
        let fd = stream.as_raw_fd();
        #[repr(C)]
        struct XuCred {
            cr_version: u32,
            cr_uid: u32,
            cr_ngroups: i16,
            cr_groups: [u32; 16],
        }
        let mut cred: XuCred = unsafe { std::mem::zeroed() };
        cred.cr_version = 0; // XUCRED_VERSION
        let mut len = std::mem::size_of::<XuCred>() as libc::socklen_t;
        const LOCAL_PEERCRED: libc::c_int = 1;
        const LOCAL_PEERPID: libc::c_int = 2;

        let ret = unsafe {
            libc::getsockopt(
                fd,
                0, // SOL_LOCAL = 0 on macOS
                LOCAL_PEERCRED,
                &mut cred as *mut _ as *mut libc::c_void,
                &mut len,
            )
        };
        if ret != 0 {
            return None;
        }

        // Also fetch PID
        let mut pid: libc::pid_t = 0;
        let mut pid_len = std::mem::size_of::<libc::pid_t>() as libc::socklen_t;
        let ret_pid = unsafe {
            libc::getsockopt(
                fd,
                0, // SOL_LOCAL = 0 on macOS
                LOCAL_PEERPID,
                &mut pid as *mut _ as *mut libc::c_void,
                &mut pid_len,
            )
        };

        Some(Self {
            uid: cred.cr_uid,
            gid: cred.cr_groups[0],
            pid: if ret_pid == 0 { Some(pid) } else { None },
        })
    }
}

/// RFC-0049: Daemon Lock Manager for fs-independent flock virtualization
/// Maintains lock state for VFS paths to support parallel build coordination
struct LockManager {
    // Map: Absolute Path -> Lock State
    locks: Mutex<HashMap<String, LockState>>,
}

struct LockState {
    // Exclusive owner (PID)
    exclusive: Option<u32>,
    // Shared owners (Set of PIDs)
    shared: HashSet<u32>,
    // Waiters notification
    notify: Arc<tokio::sync::Notify>,
}

impl LockManager {
    fn new() -> Self {
        Self {
            locks: Mutex::new(HashMap::new()),
        }
    }

    // Try to acquire lock. Returns:
    // Ok(true)  -> Granted
    // Ok(false) -> Blocked (must wait)
    // Err(_)    -> Error
    fn try_acquire(&self, path: &str, pid: u32, op: i32) -> Result<bool, String> {
        let mut locks = self.locks.lock().unwrap();
        let state = locks.entry(path.to_string()).or_insert_with(|| LockState {
            exclusive: None,
            shared: HashSet::new(),
            notify: Arc::new(tokio::sync::Notify::new()),
        });

        let is_ex = (op & libc::LOCK_EX) != 0;
        let is_sh = (op & libc::LOCK_SH) != 0;

        if is_ex {
            // Exclusive lock requires: no exclusive owner AND no shared owners (except self)
            if state.exclusive.is_some() && state.exclusive != Some(pid) {
                return Ok(false);
            }
            if !state.shared.is_empty() && (state.shared.len() > 1 || !state.shared.contains(&pid))
            {
                return Ok(false);
            }
            // Grant exclusive
            state.exclusive = Some(pid);
            state.shared.remove(&pid); // Upgrade clears shared
            Ok(true)
        } else if is_sh {
            // Shared lock requires: no exclusive owner (except self)
            if state.exclusive.is_some() && state.exclusive != Some(pid) {
                return Ok(false);
            }
            // Grant shared
            if state.exclusive == Some(pid) {
                state.exclusive = None; // Downgrade
            }
            state.shared.insert(pid);
            Ok(true)
        } else {
            Err("Invalid lock operation".to_string())
        }
    }

    fn release(&self, path: &str, pid: u32) {
        let mut locks = self.locks.lock().unwrap();
        if let Some(state) = locks.get_mut(path) {
            if state.exclusive == Some(pid) {
                state.exclusive = None;
            }
            state.shared.remove(&pid);
            // If resource is free, notify waiters
            if state.exclusive.is_none() && state.shared.is_empty() {
                state.notify.notify_waiters();
            } else if state.exclusive.is_none() {
                // If only shared locks remain, notify waiters (allowing other shared locks)
                state.notify.notify_waiters();
            }
        }
    }

    fn get_notify(&self, path: &str) -> Arc<tokio::sync::Notify> {
        let mut locks = self.locks.lock().unwrap();
        let state = locks.entry(path.to_string()).or_insert_with(|| LockState {
            exclusive: None,
            shared: HashSet::new(),
            notify: Arc::new(tokio::sync::Notify::new()),
        });
        state.notify.clone()
    }
}

/// Phase 1.1: Tracks a spawned vDird subprocess for a project
struct VDirdProcess {
    project_root: PathBuf,
    project_id: String,
    socket_path: PathBuf,
    vdir_mmap_path: PathBuf,
    #[allow(dead_code)] // will be used for process lifecycle management
    child_pid: u32,
}

struct DaemonState {
    // In-memory index of CAS blobs (Hash -> Size) - Shared across all workspaces for global dedup
    cas_index: Mutex<HashMap<[u8; 32], u64>>,
    // Per-project vDird subprocess tracking
    vdird_processes: Mutex<HashMap<PathBuf, Arc<VDirdProcess>>>,
    // Content-Addressable Storage store
    cas: vrift_cas::CasStore,
    // Lock Manager for flock virtualization
    lock_manager: LockManager,
    // Daemon start time (for uptime reporting)
    start_time: std::time::Instant,
}

async fn start_daemon() -> Result<()> {
    tracing::info!("vriftd: Starting multi-tenant daemon...");

    let cfg = vrift_config::Config::load().unwrap_or_else(|e| {
        tracing::warn!("Config load failed: {}. Using defaults.", e);
        vrift_config::Config::default()
    });
    let socket_str = cfg.socket_path().to_string_lossy().to_string();
    let path = Path::new(&socket_str);

    // Ensure socket parent directory exists
    if let Some(parent) = path.parent() {
        if !parent.exists() {
            std::fs::create_dir_all(parent)?;
        }
    }

    if path.exists() {
        tokio::fs::remove_file(path).await?;
    }

    let listener = UnixListener::bind(path)?;
    tracing::info!("vriftd: Listening on {}", socket_str);

    // Initialize shared state
    // RFC-0050: VR_THE_SOURCE via unified Config SSOT
    let cas_root_str = cfg.cas_root().display().to_string();
    let cas_root = vrift_manifest::normalize_path(&cas_root_str);
    let cas = vrift_cas::CasStore::new(&cas_root)?;

    let state = Arc::new(DaemonState {
        cas_index: Mutex::new(HashMap::new()),
        vdird_processes: Mutex::new(HashMap::new()),
        cas: cas.clone(),
        lock_manager: LockManager::new(),
        start_time: std::time::Instant::now(),
    });

    // Start background scan (Warm-up)
    let scan_state = state.clone();
    let cas_root_capture = cas_root_str.clone();
    tokio::spawn(async move {
        tracing::info!("vriftd: Starting global CAS warm-up scan...");
        if let Err(e) = scan_cas_root(&scan_state, &cas_root_capture).await {
            tracing::error!("vriftd: CAS scan failed: {}", e);
        } else {
            let _count = scan_state.cas_index.lock().unwrap().len();
            tracing::info!("vriftd: CAS warm-up complete. Indexed {} blobs.", _count);
        }
    });

    // RFC-0043: Warm up all registered workspaces on start so mmaps are ready for shims
    // TEMPORARILY DISABLED: Investigating UE blocking issues
    // {
    //     for project_root in load_registered_workspaces() {
    //         let state_clone = state.clone();
    //         tokio::spawn(async move {
    //             if let Err(e) = get_or_create_workspace(&state_clone, project_root).await {
    //                 tracing::warn!("Failed to warm up workspace: {}", e);
    //             }
    //         });
    //     }
    // }

    loop {
        tokio::select! {
            accept_result = listener.accept() => {
                match accept_result {
                    Ok((stream, _addr)) => {
                        let state = state.clone();
                        tokio::spawn(handle_connection(stream, state));
                    }
                    Err(err) => {
                        tracing::error!("vriftd: Accept error: {}", err);
                    }
                }
            }
            _ = signal::ctrl_c() => {
                println!("vriftd: Shutdown signal received");
                break;
            }
        }
    }

    println!("vriftd: Shutting down");
    if path.exists() {
        tokio::fs::remove_file(path).await?;
    }

    Ok(())
}

async fn handle_connection(mut stream: UnixStream, state: Arc<DaemonState>) {
    tracing::info!("[DAEMON] New connection accepted");
    let peer_creds = PeerCredentials::from_stream(&stream);
    let daemon_uid = unsafe { libc::getuid() };
    let mut current_vdird: Option<Arc<VDirdProcess>> = None;

    loop {
        tracing::debug!("[DAEMON] Waiting for request...");

        // Read request using v3 frame protocol
        let (header, req) = match vrift_ipc::frame_async::read_request(&mut stream).await {
            Ok(result) => result,
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
                tracing::debug!("[DAEMON] Connection closed (EOF)");
                return;
            }
            Err(e) => {
                tracing::warn!("[DAEMON] Failed to read request: {}", e);
                return;
            }
        };

        let seq_id = header.seq_id;
        tracing::debug!(
            "[DAEMON] Request received: seq_id={}, len={}",
            seq_id,
            header.length
        );

        let response = {
            tracing::info!(
                "[DAEMON] Processing request: {:?}",
                std::mem::discriminant(&req)
            );
            let resp =
                handle_request(req, &state, peer_creds, daemon_uid, &mut current_vdird).await;
            tracing::info!(
                "[DAEMON] Request processed, response: {:?}",
                std::mem::discriminant(&resp)
            );
            resp
        };

        // Send response using v3 frame protocol
        tracing::debug!("[DAEMON] Sending response (seq_id={})...", seq_id);
        if let Err(e) = vrift_ipc::frame_async::send_response(&mut stream, &response, seq_id).await
        {
            tracing::warn!("[DAEMON] Failed to send response: {}", e);
            return;
        }
        tracing::debug!("[DAEMON] Response sent successfully");
    }
}

async fn handle_request(
    req: VeloRequest,
    state: &DaemonState,
    peer_creds: Option<PeerCredentials>,
    daemon_uid: u32,
    current_vdird: &mut Option<Arc<VDirdProcess>>,
) -> VeloResponse {
    tracing::debug!("Received request: {:?}", req);
    match req {
        VeloRequest::Handshake {
            client_version: _,
            protocol_version,
        } => VeloResponse::HandshakeAck {
            server_version: env!("CARGO_PKG_VERSION").to_string(),
            protocol_version: vrift_ipc::PROTOCOL_VERSION,
            compatible: vrift_ipc::is_version_compatible(protocol_version),
        },
        VeloRequest::Status => {
            let blob_count = state.cas_index.lock().unwrap().len();
            let vdird_count = state.vdird_processes.lock().unwrap().len();
            let uptime = state.start_time.elapsed();
            let uptime_str = if uptime.as_secs() >= 3600 {
                format!(
                    "{}h{}m",
                    uptime.as_secs() / 3600,
                    (uptime.as_secs() % 3600) / 60
                )
            } else if uptime.as_secs() >= 60 {
                format!("{}m{}s", uptime.as_secs() / 60, uptime.as_secs() % 60)
            } else {
                format!("{}s", uptime.as_secs())
            };
            VeloResponse::StatusAck {
                status: format!(
                    "Multi-tenant Operational (Global Blobs: {}, vDird Processes: {}, Uptime: {})",
                    blob_count, vdird_count, uptime_str
                ),
            }
        }
        VeloRequest::RegisterWorkspace {
            project_root: root_str,
        } => {
            let project_root = PathBuf::from(&root_str)
                .canonicalize()
                .unwrap_or_else(|_| PathBuf::from(&root_str));
            tracing::info!(
                "vriftd: Workspace Registration (canonicalized): root={:?}",
                project_root
            );
            if !project_root.exists() {
                tracing::error!(
                    "vriftd: Registration failed - root does not exist: '{:?}'",
                    project_root
                );
                return VeloResponse::Error(VeloError::not_found("Project root does not exist"));
            }

            match spawn_or_get_vdird(state, project_root).await {
                Ok(vdird) => {
                    tracing::info!(
                        "vriftd: Workspace registered: id={}, socket={:?}, root={:?}",
                        vdird.project_id,
                        vdird.socket_path,
                        vdird.project_root
                    );
                    *current_vdird = Some(vdird.clone());
                    VeloResponse::RegisterAck {
                        workspace_id: vdird.project_id.clone(),
                        vdird_socket: vdird.socket_path.to_string_lossy().to_string(),
                        vdir_mmap_path: vdird.vdir_mmap_path.to_string_lossy().to_string(),
                    }
                }
                Err(e) => {
                    tracing::error!("vriftd: Workspace registration failed: {}", e);
                    VeloResponse::Error(VeloError::internal(format!(
                        "Workspace registration failed: {}",
                        e
                    )))
                }
            }
        }
        VeloRequest::Spawn { command, env, cwd } => {
            if let Some(creds) = peer_creds {
                if creds.uid != daemon_uid && creds.uid != 0 {
                    return VeloResponse::Error(VeloError::permission_denied("UID mismatch"));
                }
            } else {
                return VeloResponse::Error(VeloError::permission_denied("Verification failed"));
            }
            handle_spawn(command, env, cwd).await
        }
        VeloRequest::CasInsert { hash, size } => {
            let mut index = state.cas_index.lock().unwrap();
            index.insert(hash, size);
            VeloResponse::CasAck
        }
        VeloRequest::CasGet { hash } => {
            let index = state.cas_index.lock().unwrap();
            if let Some(&size) = index.get(&hash) {
                VeloResponse::CasFound { size }
            } else {
                VeloResponse::CasNotFound
            }
        }
        VeloRequest::Protect {
            path,
            immutable,
            owner,
        } => {
            // Sandboxing check using centralized path utilities
            if let Some(ref vdird) = current_vdird {
                if !is_within_directory(&path, &vdird.project_root) {
                    return VeloResponse::Error(VeloError::permission_denied(
                        "Path outside project root",
                    ));
                }
            } else {
                return VeloResponse::Error(VeloError::workspace_not_registered());
            }
            handle_protect(path, immutable, owner).await
        }
        // Phase 1.1: Manifest operations are now handled by vDird subprocess.
        // Clients should route these to the vDird socket returned in RegisterAck.
        VeloRequest::ManifestGet { path } => {
            tracing::warn!(
                "vriftd: ManifestGet '{}' received — route to vDird instead",
                path
            );
            VeloResponse::Error(VeloError::new(
                VeloErrorKind::WorkspaceNotRegistered,
                "Manifest operations must be routed to vDird. Use the vdird_socket from RegisterAck.",
            ))
        }
        VeloRequest::ManifestUpsert { path, .. } => {
            tracing::warn!(
                "vriftd: ManifestUpsert '{}' received — route to vDird instead",
                path
            );
            VeloResponse::Error(VeloError::new(
                VeloErrorKind::WorkspaceNotRegistered,
                "Manifest operations must be routed to vDird. Use the vdird_socket from RegisterAck.",
            ))
        }
        VeloRequest::ManifestRemove { path } => {
            tracing::warn!(
                "vriftd: ManifestRemove '{}' received — route to vDird instead",
                path
            );
            VeloResponse::Error(VeloError::new(
                VeloErrorKind::WorkspaceNotRegistered,
                "Manifest operations must be routed to vDird. Use the vdird_socket from RegisterAck.",
            ))
        }
        VeloRequest::ManifestRename { old_path, .. } => {
            tracing::warn!(
                "vriftd: ManifestRename '{}' received — route to vDird instead",
                old_path
            );
            VeloResponse::Error(VeloError::new(
                VeloErrorKind::WorkspaceNotRegistered,
                "Manifest operations must be routed to vDird. Use the vdird_socket from RegisterAck.",
            ))
        }
        VeloRequest::ManifestUpdateMtime { path, .. } => {
            tracing::warn!(
                "vriftd: ManifestUpdateMtime '{}' received — route to vDird instead",
                path
            );
            VeloResponse::Error(VeloError::new(
                VeloErrorKind::WorkspaceNotRegistered,
                "Manifest operations must be routed to vDird. Use the vdird_socket from RegisterAck.",
            ))
        }
        VeloRequest::ManifestReingest { vpath, .. } => {
            tracing::warn!(
                "vriftd: ManifestReingest '{}' received — route to vDird instead",
                vpath
            );
            VeloResponse::Error(VeloError::new(
                VeloErrorKind::WorkspaceNotRegistered,
                "Manifest operations must be routed to vDird. Use the vdird_socket from RegisterAck.",
            ))
        }
        // RFC-0049: Flock Virtualization
        VeloRequest::FlockAcquire { path, operation } => {
            // PID required for locking
            let pid = match peer_creds.and_then(|c| c.pid) {
                Some(p) => p as u32,
                None => {
                    return VeloResponse::Error(VeloError::internal(
                        "Could not determine PID for lock",
                    ))
                }
            };

            // Loop until acquired or error
            loop {
                match state.lock_manager.try_acquire(&path, pid, operation) {
                    Ok(true) => return VeloResponse::FlockAck,
                    Ok(false) => {
                        // Blocked
                        if operation & libc::LOCK_NB != 0 {
                            // Non-blocking request
                            return VeloResponse::Error(VeloError::new(
                                VeloErrorKind::LockFailed,
                                "EWOULDBLOCK",
                            ));
                        }
                        // Blocking request: wait for notification
                        let notify = state.lock_manager.get_notify(&path);
                        notify.notified().await;
                        // Retry loop after notification
                    }
                    Err(e) => {
                        return VeloResponse::Error(VeloError::new(VeloErrorKind::LockFailed, e))
                    }
                }
            }
        }
        VeloRequest::FlockRelease { path } => {
            let pid = match peer_creds.and_then(|c| c.pid) {
                Some(p) => p as u32,
                None => {
                    return VeloResponse::Error(VeloError::internal(
                        "Could not determine PID for unlock",
                    ))
                }
            };
            state.lock_manager.release(&path, pid);
            VeloResponse::FlockAck
        }
        VeloRequest::CasSweep { bloom_filter } => {
            match state.cas.sweep(&bloom_filter) {
                Ok((deleted_count, reclaimed_bytes)) => {
                    // Update global index
                    let mut index = state.cas_index.lock().unwrap();
                    index.clear();
                    if let Ok(iter) = state.cas.iter() {
                        for hash in iter.flatten() {
                            if let Some(path) = state.cas.blob_path_for_hash(&hash) {
                                if let Ok(meta) = std::fs::metadata(path) {
                                    index.insert(hash, meta.len());
                                }
                            }
                        }
                    }
                    VeloResponse::CasSweepAck {
                        deleted_count,
                        reclaimed_bytes,
                    }
                }
                Err(e) => VeloResponse::Error(VeloError::internal(format!("Sweep failed: {}", e))),
            }
        }
        VeloRequest::ManifestListDir { path } => {
            tracing::warn!(
                "vriftd: ManifestListDir '{}' received — route to vDird instead",
                path
            );
            VeloResponse::Error(VeloError::new(
                VeloErrorKind::WorkspaceNotRegistered,
                "Manifest operations must be routed to vDird. Use the vdird_socket from RegisterAck.",
            ))
        }
        // IngestFullScan: Unified ingest architecture
        // CLI becomes thin client, daemon handles all ingest logic
        VeloRequest::IngestFullScan {
            path,
            manifest_path,
            threads,
            phantom,
            tier1,
            prefix,
        } => {
            use std::time::Instant;
            use vrift_cas::{streaming_ingest, IngestMode};

            let source_path = PathBuf::from(&path);
            let manifest_out = PathBuf::from(&manifest_path);

            tracing::info!(
                path = %path,
                manifest = %manifest_path,
                threads = ?threads,
                phantom = phantom,
                tier1 = tier1,
                prefix = ?prefix,
                "Starting streaming ingest"
            );

            let start = Instant::now();

            // Determine mode
            let mode = if phantom {
                IngestMode::Phantom
            } else if tier1 {
                IngestMode::SolidTier1
            } else {
                IngestMode::SolidTier2
            };

            // Use configured CAS path
            let cas_root_path = state.cas.root().to_path_buf();

            // Run streaming ingest in blocking task
            let source_clone = source_path.clone();
            let cas_clone = cas_root_path.clone();
            let results = match tokio::task::spawn_blocking(move || {
                tracing::info!("spawn_blocking: starting streaming_ingest");
                let r = streaming_ingest(&source_clone, &cas_clone, mode, threads);
                tracing::info!("spawn_blocking: streaming_ingest done, {} results", r.len());
                r
            })
            .await
            {
                Ok(r) => r,
                Err(e) => {
                    return VeloResponse::Error(VeloError::new(
                        VeloErrorKind::IngestFailed,
                        format!("Ingest task failed: {}", e),
                    ))
                }
            };

            let total_files = results.len() as u64;

            // 5. Collect stats
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

            // 6. Write LMDB manifest (RFC-0039 compatible with shim)
            if let Err(e) = write_ingest_manifest(
                &manifest_out,
                &source_path,
                &results,
                tier1,
                prefix.as_deref(),
            ) {
                return VeloResponse::Error(VeloError::io_error(format!(
                    "Failed to write manifest: {}",
                    e
                )));
            }

            tracing::info!(
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
                manifest_path,
            }
        }
    }
}

/// Write manifest file from ingest results using LMDB format
/// (RFC-0039: Compatible with cmd_ingest and shim)
fn write_ingest_manifest(
    manifest_path: &Path,
    source_root: &Path,
    results: &[Result<vrift_cas::IngestResult, vrift_cas::CasError>],
    tier1: bool,
    prefix: Option<&str>,
) -> Result<()> {
    use std::os::unix::fs::MetadataExt;
    use vrift_manifest::VnodeEntry;

    // Open or create LMDB manifest
    let manifest = LmdbManifest::open(manifest_path)?;

    // Determine asset tier
    let asset_tier = if tier1 {
        AssetTier::Tier1Immutable
    } else {
        AssetTier::Tier2Mutable
    };

    for result in results.iter().flatten() {
        // Get file metadata for mtime and mode
        let metadata = match std::fs::metadata(&result.source_path) {
            Ok(m) => m,
            Err(_) => {
                // File might have been moved (phantom mode) - try symlink metadata
                match std::fs::symlink_metadata(&result.source_path) {
                    Ok(m) => m,
                    Err(_) => continue, // Skip files we can't stat
                }
            }
        };

        // Compute manifest path: relative to source_root with optional prefix
        let canon_source = result
            .source_path
            .canonicalize()
            .unwrap_or_else(|_| result.source_path.clone());
        let canon_root = source_root
            .canonicalize()
            .unwrap_or_else(|_| source_root.to_path_buf());

        let relative_path = canon_source
            .strip_prefix(&canon_root)
            .unwrap_or(&canon_source);

        // RFC-0050: Apply prefix correctly
        let prefix_str = prefix.unwrap_or("");
        let manifest_key = if prefix_str == "/" {
            format!("/{}", relative_path.display())
        } else if prefix_str.is_empty() {
            // Default behavior or empty prefix: relative path with leading /
            format!("/{}", relative_path.display())
        } else {
            // User provided a custom prefix, e.g. "/vrift"
            format!(
                "{}/{}",
                prefix_str.trim_end_matches('/'),
                relative_path.display()
            )
        };

        // Extract mtime and mode
        let mtime = metadata.mtime() as u64;
        let mode = metadata.mode();

        // Create VnodeEntry
        let vnode = VnodeEntry::new_file(result.hash, result.size, mtime, mode);

        // Insert into LMDB manifest
        manifest.insert(&manifest_key, vnode, asset_tier);
    }

    // Commit delta layer to LMDB base layer (required for persistence!)
    manifest.commit()?;

    // Phase 1.1: mmap cache is now managed by vDird subprocess, not vriftd

    Ok(())
}

async fn handle_protect(path_str: String, immutable: bool, owner: Option<String>) -> VeloResponse {
    // Security: Path sandboxing - reject suspicious paths
    if path_str.contains("..") || path_str.contains('\0') {
        return VeloResponse::Error(VeloError::invalid_path("Path traversal detected"));
    }

    let path = Path::new(&path_str);

    // Canonicalize to resolve symlinks and validate existence
    let canonical = match path.canonicalize() {
        Ok(p) => p,
        Err(_) => {
            return VeloResponse::Error(VeloError::not_found(format!(
                "Path not found: {}",
                path_str
            )))
        }
    };

    // Additional check: ensure canonicalized path doesn't escape expected directories
    let canonical_str = canonical.to_string_lossy();
    if canonical_str.contains("..") {
        return VeloResponse::Error(VeloError::invalid_path(
            "Canonicalized path contains traversal",
        ));
    }

    // 1. Set immutable flag via vrift-cas::protection
    if let Err(e) = vrift_cas::protection::set_immutable(&canonical, immutable) {
        tracing::warn!("Failed to set immutable flag on {}: {}", canonical_str, e);
        // We continue anyway, as ownership might still work
    }

    // 2. Set ownership if requested (Requires root/CAP_CHOWN if daemon is privileged)
    if let Some(user) = owner {
        #[cfg(unix)]
        {
            use nix::unistd::{chown, User};
            if let Ok(Some(u)) = User::from_name(&user) {
                if let Err(e) = chown(&canonical, Some(u.uid), Some(u.gid)) {
                    tracing::error!("Failed to chown {} to {}: {}", canonical_str, user, e);
                    return VeloResponse::Error(VeloError::permission_denied(format!(
                        "chown failed: {}",
                        e
                    )));
                }
            } else {
                return VeloResponse::Error(VeloError::not_found(format!(
                    "User not found: {}",
                    user
                )));
            }
        }
    }

    VeloResponse::ProtectAck
}

async fn handle_spawn(
    command: Vec<String>,
    env: Vec<(String, String)>,
    cwd: String,
) -> VeloResponse {
    if command.is_empty() {
        return VeloResponse::Error(VeloError::internal("Command cannot be empty"));
    }

    // For MVP, we just spawn the process and let it run detached
    // In a real system, we'd track it in a ProcessManager struct
    println!("Spawning: {:?} in {}", command, cwd);

    let mut cmd = tokio::process::Command::new(&command[0]);
    cmd.args(&command[1..]);
    cmd.envs(env);
    cmd.current_dir(cwd);

    // We direct stdout/stderr to inherit for now, so they appear in daemon logs
    // Ideally we would capture or stream them
    // cmd.stdout(std::process::Stdio::inherit());
    // cmd.stderr(std::process::Stdio::inherit());

    match cmd.spawn() {
        Ok(child) => {
            let pid = child.id().unwrap_or(0);
            println!("Spawned PID: {}", pid);

            // Important: Avoid zombie processes.
            // Since we're not waiting for it here (async handling), we drop the Child handle.
            // But tokio::process::Command spawns are automatically reaped by tokio runtime if we don't await?
            // Actually, we SHOULD store the child handle if we want to manage it.
            // For this MVP step 1, we'll let it run.
            tokio::spawn(async move {
                let _ = child.wait_with_output().await;
            });

            VeloResponse::SpawnAck { pid }
        }
        Err(e) => VeloResponse::Error(VeloError::internal(format!("Failed to spawn: {}", e))),
    }
}

async fn scan_cas_root(state: &DaemonState, cas_root_path: &str) -> Result<()> {
    let cas_root = vrift_manifest::normalize_path(cas_root_path);

    if !cas_root.exists() {
        tracing::warn!(
            "vriftd: CAS root not found at {:?}, skipping scan.",
            cas_root
        );
        return Ok(());
    }

    use vrift_cas::CasStore;
    let cas = CasStore::new(cas_root)?;

    // We can use CasStore's iterator, but it's synchronous (blocking).
    // For now, we'll wrap it in spawn_blocking or just run it since we are in a dedicated task.
    // Iterating millions of files might take time, so blocking the runtime is bad if not careful.
    // But this is a separate task.

    let mut index = state.cas_index.lock().unwrap();

    // Using blocking iterator
    for hash in (cas.iter()?).flatten() {
        // For size, we currently don't store it in the filename, so we might need to stat.
        // Statting every file is expensive.
        // For MVP, if we don't have size efficiently, we can put 0 or Stat content.
        // Optimized Velo stores [hash_prefix]/[hash] and we can trust it exists.
        if let Some(path) = cas.blob_path_for_hash(&hash) {
            if let Ok(metadata) = std::fs::metadata(path) {
                index.insert(hash, metadata.len());
            }
        }
    }

    Ok(())
}

/// Phase 1.1: Spawn or reuse a vDird subprocess for the given project root.
/// vDird handles all manifest operations, VDir mmap, and fs watching.
async fn spawn_or_get_vdird(
    state: &DaemonState,
    project_root: PathBuf,
) -> Result<Arc<VDirdProcess>> {
    // Check if already running
    {
        let processes = state.vdird_processes.lock().unwrap();
        if let Some(vdird) = processes.get(&project_root) {
            // Verify socket still exists (basic health check)
            if vdird.socket_path.exists() {
                return Ok(vdird.clone());
            }
            tracing::warn!(
                "vriftd: vDird socket missing for {:?}, respawning...",
                project_root
            );
        }
    }

    tracing::info!("vriftd: Spawning vDird for: {:?}", project_root);

    // Compute project ID and paths
    let project_id = vrift_config::path::compute_project_id(&project_root);
    let socket_path = vrift_config::path::get_vdird_socket_path(&project_id).unwrap_or_else(|| {
        let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
        PathBuf::from(home)
            .join(".vrift")
            .join("sockets")
            .join(format!("{}.sock", &project_id[..16.min(project_id.len())]))
    });
    let vdir_mmap_path = vrift_config::path::get_vdir_mmap_path(&project_id)
        .unwrap_or_else(|| project_root.join(".vrift").join("vdir.mmap"));

    // Ensure socket parent directory exists
    if let Some(parent) = socket_path.parent() {
        if !parent.exists() {
            std::fs::create_dir_all(parent)?;
        }
    }

    // Find vdir_d binary: same directory as vriftd, then PATH
    let vdird_bin = find_vdird_binary()?;

    // Spawn vDird subprocess
    let child = std::process::Command::new(&vdird_bin)
        .arg(project_root.to_string_lossy().as_ref())
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|e| anyhow::anyhow!("Failed to spawn vDird: {}", e))?;

    let child_pid = child.id();
    tracing::info!(
        "vriftd: vDird spawned: pid={}, socket={:?}",
        child_pid,
        socket_path
    );

    // Wait for vDird socket to appear (poll with timeout)
    let max_wait = std::time::Duration::from_secs(10);
    let poll_interval = std::time::Duration::from_millis(100);
    let start = std::time::Instant::now();
    while !socket_path.exists() {
        if start.elapsed() > max_wait {
            return Err(anyhow::anyhow!(
                "vDird socket did not appear within {:?}: {:?}",
                max_wait,
                socket_path
            ));
        }
        tokio::time::sleep(poll_interval).await;
    }

    tracing::info!(
        "vriftd: vDird ready: pid={}, socket={:?}",
        child_pid,
        socket_path
    );

    let vdird = Arc::new(VDirdProcess {
        project_root: project_root.clone(),
        project_id,
        socket_path,
        vdir_mmap_path,
        child_pid,
    });

    let mut processes = state.vdird_processes.lock().unwrap();
    processes.insert(project_root, vdird.clone());

    Ok(vdird)
}

/// Find the vdir_d binary. Looks in same directory as vriftd, then falls back to PATH.
fn find_vdird_binary() -> Result<PathBuf> {
    let current_exe = std::env::current_exe()?;
    if let Some(bin_dir) = current_exe.parent() {
        let candidate = bin_dir.join("vdir_d");
        if candidate.exists() {
            return Ok(candidate);
        }
    }

    // Fallback: search PATH
    if let Ok(output) = std::process::Command::new("which").arg("vdir_d").output() {
        if output.status.success() {
            let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !path.is_empty() {
                return Ok(PathBuf::from(path));
            }
        }
    }

    Err(anyhow::anyhow!(
        "Could not find vdir_d binary. Ensure it is built and in the same directory as vriftd."
    ))
}
