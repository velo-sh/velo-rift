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
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let cli = Cli::parse();

    match cli.command.unwrap_or(Commands::Start) {
        Commands::Start => start_daemon().await?,
    }

    Ok(())
}

use std::path::{Path, PathBuf};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{UnixListener, UnixStream};
use vrift_ipc::{VeloRequest, VeloResponse};
use vrift_manifest::lmdb::{AssetTier, LmdbManifest};

// RFC-0043: Minimal registry for workspace discovery
#[derive(serde::Deserialize)]
struct MinimalManifestEntry {
    project_root: PathBuf,
}

#[derive(serde::Deserialize)]
struct MinimalRegistry {
    manifests: std::collections::HashMap<String, MinimalManifestEntry>,
}

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

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::Mutex;

use vrift_ipc::{bloom_hashes, BLOOM_SIZE};

const MAX_IPC_SIZE: usize = 16 * 1024 * 1024; // 16 MB max to prevent DoS

#[derive(Debug, Clone, Copy)]
struct PeerCredentials {
    uid: u32,
    #[allow(dead_code)]
    gid: u32,
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
        let ret = unsafe {
            libc::getsockopt(
                fd,
                0, // SOL_LOCAL = 0 on macOS
                LOCAL_PEERCRED,
                &mut cred as *mut _ as *mut libc::c_void,
                &mut len,
            )
        };
        if ret == 0 {
            Some(Self {
                uid: cred.cr_uid,
                gid: cred.cr_groups[0],
            })
        } else {
            None
        }
    }
}

struct BloomFilter {
    shm_ptr: *mut u8,
}

unsafe impl Send for BloomFilter {}
unsafe impl Sync for BloomFilter {}

impl BloomFilter {
    fn new(shm_ptr: *mut u8) -> Self {
        Self { shm_ptr }
    }

    fn clear(&self) {
        unsafe { std::ptr::write_bytes(self.shm_ptr, 0, BLOOM_SIZE) };
    }

    fn add(&self, path: &str) {
        let (h1, h2) = self.hashes(path);
        let b1 = h1 % (BLOOM_SIZE * 8);
        let b2 = h2 % (BLOOM_SIZE * 8);
        unsafe {
            let p1 = self.shm_ptr.add(b1 / 8);
            *p1 |= 1 << (b1 % 8);
            let p2 = self.shm_ptr.add(b2 / 8);
            *p2 |= 1 << (b2 % 8);
        }
    }

    fn hashes(&self, s: &str) -> (usize, usize) {
        bloom_hashes(s)
    }
}

struct WorkspaceState {
    project_root: PathBuf,
    // VFS Manifest (LMDB-backed for ACID persistence)
    manifest: std::sync::Mutex<LmdbManifest>,
    bloom: BloomFilter,
    // Offset in Bloom Filter shared memory is handled by WorkspaceState
    shm_name: String,
}

struct DaemonState {
    // In-memory index of CAS blobs (Hash -> Size) - Shared across all workspaces for global dedup
    cas_index: Mutex<HashMap<[u8; 32], u64>>,
    // Workspaces indexed by project root path
    workspaces: Mutex<HashMap<PathBuf, Arc<WorkspaceState>>>,
    // Content-Addressable Storage store
    cas: vrift_cas::CasStore,
}

/// RFC-0044 Hot Stat Cache: Export manifest to mmap file for O(1) shim access
fn export_mmap_cache(manifest: &LmdbManifest, project_root: &Path) {
    use vrift_ipc::ManifestMmapBuilder;

    let mut builder = ManifestMmapBuilder::new();

    // Mmap path in project's .vrift directory for consistent shim lookup
    let vrift_dir = project_root.join(".vrift");
    if !vrift_dir.exists() {
        if let Err(e) = std::fs::create_dir_all(&vrift_dir) {
            tracing::warn!("Failed to create .vrift dir for mmap: {}", e);
            return;
        }
    }
    let mmap_path = vrift_dir.join("manifest.mmap");
    let mmap_path_str = mmap_path.to_string_lossy();

    // Iterate all manifest entries and add to builder
    if let Ok(entries) = manifest.iter() {
        for (path, entry) in entries {
            let flags = entry.vnode.mode;
            let is_dir = entry.vnode.is_dir();
            let is_symlink = entry.vnode.is_symlink();
            builder.add_entry(
                &path,
                entry.vnode.size,
                entry.vnode.mtime as i64,
                flags,
                is_dir,
                is_symlink,
            );
        }
    }

    if builder.is_empty() {
        return;
    }

    // Write mmap file atomically
    match builder.write_to_file(&mmap_path_str) {
        Ok(()) => {
            tracing::info!(
                "RFC-0044 Hot Stat Cache: Exported {} entries to {}",
                builder.len(),
                mmap_path_str
            );
        }
        Err(e) => {
            tracing::warn!("Failed to export mmap cache to {}: {}", mmap_path_str, e);
        }
    }
}

async fn start_daemon() -> Result<()> {
    tracing::info!("vriftd: Starting multi-tenant daemon...");

    let socket_path = "/tmp/vrift.sock";
    let path = Path::new(socket_path);

    if path.exists() {
        tokio::fs::remove_file(path).await?;
    }

    let listener = UnixListener::bind(path)?;
    tracing::info!("vriftd: Listening on {}", socket_path);

    // Initialize shared state
    let cas_root_str =
        std::env::var("VRIFT_CAS_ROOT").unwrap_or_else(|_| "~/.vrift/cas".to_string());
    let cas_root = vrift_manifest::normalize_path(&cas_root_str);
    let cas = vrift_cas::CasStore::new(&cas_root)?;

    let state = Arc::new(DaemonState {
        cas_index: Mutex::new(HashMap::new()),
        workspaces: Mutex::new(HashMap::new()),
        cas: cas.clone(),
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
    {
        for project_root in load_registered_workspaces() {
            let state_clone = state.clone();
            tokio::spawn(async move {
                if let Err(e) = get_or_create_workspace(&state_clone, project_root).await {
                    tracing::warn!("Failed to warm up workspace: {}", e);
                }
            });
        }
    }

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
    let peer_creds = PeerCredentials::from_stream(&stream);
    let daemon_uid = unsafe { libc::getuid() };
    let mut current_workspace: Option<Arc<WorkspaceState>> = None;

    loop {
        let mut len_buf = [0u8; 4];
        if stream.read_exact(&mut len_buf).await.is_err() {
            return;
        }
        let len = u32::from_le_bytes(len_buf) as usize;

        if len > MAX_IPC_SIZE {
            tracing::warn!("IPC message too large: {} bytes", len);
            return;
        }

        let mut buf = vec![0u8; len];
        if stream.read_exact(&mut buf).await.is_err() {
            return;
        }

        let response = match bincode::deserialize::<VeloRequest>(&buf) {
            Ok(req) => {
                handle_request(req, &state, peer_creds, daemon_uid, &mut current_workspace).await
            }
            Err(e) => VeloResponse::Error(format!("Invalid request: {}", e)),
        };

        let resp_bytes = bincode::serialize(&response).unwrap();
        let resp_len = (resp_bytes.len() as u32).to_le_bytes();
        if stream.write_all(&resp_len).await.is_err() {
            return;
        }
        if stream.write_all(&resp_bytes).await.is_err() {
            return;
        }
    }
}

async fn handle_request(
    req: VeloRequest,
    state: &DaemonState,
    peer_creds: Option<PeerCredentials>,
    daemon_uid: u32,
    current_workspace: &mut Option<Arc<WorkspaceState>>,
) -> VeloResponse {
    tracing::debug!("Received request: {:?}", req);
    match req {
        VeloRequest::Handshake { client_version: _ } => VeloResponse::HandshakeAck {
            server_version: env!("CARGO_PKG_VERSION").to_string(),
        },
        VeloRequest::Status => {
            let count = state.cas_index.lock().unwrap().len();
            VeloResponse::StatusAck {
                status: format!("Multi-tenant Operational (Global Blobs: {})", count),
            }
        }
        VeloRequest::RegisterWorkspace {
            project_root: root_str,
        } => {
            tracing::info!("vriftd: Workspace Registration Request for: {}", root_str);
            let project_root = PathBuf::from(&root_str);
            if !project_root.exists() {
                return VeloResponse::Error("Project root does not exist".to_string());
            }

            // Security: In a production system, verify that peer_creds has access to this folder
            // For now, we allow any local user but bind the connection to this root.
            match get_or_create_workspace(state, project_root).await {
                Ok(ws) => {
                    *current_workspace = Some(ws.clone());
                    VeloResponse::RegisterAck {
                        workspace_id: ws.shm_name.clone(),
                    }
                }
                Err(e) => VeloResponse::Error(format!("Workspace registration failed: {}", e)),
            }
        }
        VeloRequest::Spawn { command, env, cwd } => {
            if let Some(creds) = peer_creds {
                if creds.uid != daemon_uid && creds.uid != 0 {
                    return VeloResponse::Error("Permission denied: UID mismatch".to_string());
                }
            } else {
                return VeloResponse::Error("Permission denied: verification failed".to_string());
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
            // Sandboxing check
            if let Some(ref ws) = current_workspace {
                if !path.starts_with(ws.project_root.to_str().unwrap_or("")) {
                    return VeloResponse::Error(
                        "Access Denied: Path outside project root".to_string(),
                    );
                }
            } else {
                return VeloResponse::Error("Access Denied: Workspace not registered".to_string());
            }
            handle_protect(path, immutable, owner).await
        }
        VeloRequest::ManifestGet { path } => {
            if let Some(ref ws) = current_workspace {
                let manifest = ws.manifest.lock().unwrap();
                let entry = match manifest.get(&path) {
                    Ok(Some(manifest_entry)) => Some(manifest_entry.vnode.clone()),
                    _ => None,
                };
                tracing::info!(
                    "vriftd: ManifestGet lookup for '{}' -> {}",
                    path,
                    if entry.is_some() {
                        "FOUND"
                    } else {
                        "NOT FOUND"
                    }
                );
                VeloResponse::ManifestAck { entry }
            } else {
                VeloResponse::Error("Workspace not registered".to_string())
            }
        }
        VeloRequest::ManifestUpsert { path, entry } => {
            if let Some(ref ws) = current_workspace {
                let manifest = ws.manifest.lock().unwrap();
                manifest.insert(&path, entry, AssetTier::Tier2Mutable);
                ws.bloom.add(&path);
                let _ = manifest.commit();
                export_mmap_cache(&manifest, &ws.project_root);
                VeloResponse::ManifestAck { entry: None }
            } else {
                VeloResponse::Error("Workspace not registered".to_string())
            }
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
                Err(e) => VeloResponse::Error(format!("Sweep failed: {}", e)),
            }
        }
        VeloRequest::ManifestListDir { path } => {
            if let Some(ref ws) = current_workspace {
                let manifest = ws.manifest.lock().unwrap();
                let mut entries = Vec::new();
                let prefix = if path.is_empty() {
                    String::new()
                } else {
                    format!("{}/", path.trim_end_matches('/'))
                };
                let prefix_len = prefix.len();
                let mut seen = std::collections::HashSet::new();
                if let Ok(all_entries) = manifest.iter() {
                    for (entry_path, manifest_entry) in all_entries {
                        if entry_path.starts_with(&prefix) {
                            let remainder = &entry_path[prefix_len..];
                            let child_name = remainder.split('/').next().unwrap_or(remainder);
                            if !child_name.is_empty() && seen.insert(child_name.to_string()) {
                                let is_dir =
                                    manifest_entry.vnode.is_dir() || remainder.contains('/');
                                entries.push(vrift_ipc::DirEntry {
                                    name: child_name.to_string(),
                                    is_dir,
                                });
                            }
                        }
                    }
                }
                VeloResponse::ManifestListAck { entries }
            } else {
                VeloResponse::Error("Workspace not registered".to_string())
            }
        }
    }
}

async fn handle_protect(path_str: String, immutable: bool, owner: Option<String>) -> VeloResponse {
    // Security: Path sandboxing - reject suspicious paths
    if path_str.contains("..") || path_str.contains('\0') {
        return VeloResponse::Error("Invalid path: path traversal detected".to_string());
    }

    let path = Path::new(&path_str);

    // Canonicalize to resolve symlinks and validate existence
    let canonical = match path.canonicalize() {
        Ok(p) => p,
        Err(_) => return VeloResponse::Error(format!("Path not found: {}", path_str)),
    };

    // Additional check: ensure canonicalized path doesn't escape expected directories
    let canonical_str = canonical.to_string_lossy();
    if canonical_str.contains("..") {
        return VeloResponse::Error(
            "Invalid path: canonicalized path contains traversal".to_string(),
        );
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
                    return VeloResponse::Error(format!("chown failed: {}", e));
                }
            } else {
                return VeloResponse::Error(format!("User not found: {}", user));
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
        return VeloResponse::Error("Command cannot be empty".to_string());
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
        Err(e) => VeloResponse::Error(format!("Failed to spawn: {}", e)),
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

async fn get_or_create_workspace(
    state: &DaemonState,
    project_root: PathBuf,
) -> Result<Arc<WorkspaceState>> {
    let mut workspaces = state.workspaces.lock().unwrap();
    if let Some(ws) = workspaces.get(&project_root) {
        return Ok(ws.clone());
    }

    tracing::info!("Initializing new workspace for: {:?}", project_root);

    // 1. Setup LMDB manifest - use the SAME manifest.lmdb that CLI writes to
    let vrift_dir = project_root.join(".vrift");
    if !vrift_dir.exists() {
        std::fs::create_dir_all(&vrift_dir)?;
    }
    let manifest_path = vrift_dir.join("manifest.lmdb");
    let manifest = LmdbManifest::open(manifest_path.to_str().unwrap())?;

    // RFC-0039: Initial import from legacy flat manifest if this is a new workspace
    // Note: CLI now writes directly to manifest.lmdb, so this is for backwards compat
    if manifest.is_empty()? {
        let flat_path = project_root.join("vrift.manifest");
        if flat_path.exists() {
            tracing::info!("vriftd: Importing flat manifest from {:?}", flat_path);
            let flat = vrift_manifest::Manifest::load(&flat_path)?;
            for (path, vnode) in flat.iter() {
                // Using Tier2Mutable (Solid Tier-2) as the default for ingested files
                manifest.insert(path, vnode.clone(), vrift_manifest::AssetTier::Tier2Mutable);
            }
            manifest.commit()?;
            manifest.sync()?;
        }
    }

    // 2. Setup Shared Memory Bloom Filter
    use nix::fcntl::OFlag;
    use nix::sys::mman::{mmap, shm_open, shm_unlink, MapFlags, ProtFlags};
    use nix::sys::stat::Mode;

    let root_str = project_root.to_string_lossy();
    let root_hash = blake3::hash(root_str.as_bytes());
    let shm_name = format!("/vrift_bloom_{}", &root_hash.to_hex()[..16]);

    let _ = shm_unlink(shm_name.as_str());
    let shm_fd = shm_open(
        shm_name.as_str(),
        OFlag::O_CREAT | OFlag::O_RDWR,
        Mode::S_IRUSR | Mode::S_IWUSR,
    )?;
    nix::unistd::ftruncate(&shm_fd, BLOOM_SIZE as i64)?;

    let shm_ptr = unsafe {
        mmap(
            None,
            std::num::NonZeroUsize::new(BLOOM_SIZE).unwrap(),
            ProtFlags::PROT_READ | ProtFlags::PROT_WRITE,
            MapFlags::MAP_SHARED,
            &shm_fd,
            0,
        )?
    }
    .as_ptr() as *mut u8;

    let bloom = BloomFilter::new(shm_ptr);
    bloom.clear();

    // Populate bloom
    if let Ok(entries) = manifest.iter() {
        for (path, _) in entries {
            bloom.add(&path);
        }
    }

    let ws = Arc::new(WorkspaceState {
        project_root: project_root.clone(),
        manifest: std::sync::Mutex::new(manifest),
        bloom,
        shm_name,
    });

    workspaces.insert(project_root, ws.clone());

    // Export initial mmap cache
    {
        let manifest = ws.manifest.lock().unwrap();
        export_mmap_cache(&manifest, &ws.project_root);
    }

    Ok(ws)
}
