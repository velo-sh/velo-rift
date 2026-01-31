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

use std::path::Path;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{UnixListener, UnixStream};
use vrift_ipc::{VeloRequest, VeloResponse};

use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;

use vrift_ipc::{bloom_hashes, BLOOM_SIZE};
use vrift_manifest::Manifest;

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

struct DaemonState {
    // In-memory index of CAS blobs (Hash -> Size)
    cas_index: Mutex<HashMap<[u8; 32], u64>>,
    // VFS Manifest
    manifest: Mutex<Manifest>,
    manifest_path: String,
    bloom: BloomFilter,
}

async fn start_daemon() -> Result<()> {
    tracing::info!("vriftd: Starting daemon...");

    let manifest_path =
        std::env::var("VRIFT_MANIFEST").unwrap_or_else(|_| "velo.manifest".to_string());
    let manifest = Manifest::load(&manifest_path).unwrap_or_else(|_| {
        tracing::warn!(
            "Failed to load manifest from {}, creating new.",
            manifest_path
        );
        Manifest::new()
    });

    let socket_path = "/tmp/vrift.sock";
    let path = Path::new(socket_path);

    if path.exists() {
        tokio::fs::remove_file(path).await?;
    }

    let listener = UnixListener::bind(path)?;
    tracing::info!("vriftd: Listening on {}", socket_path);

    // Shared Memory Bloom Filter Setup
    use nix::fcntl::OFlag;
    use nix::sys::mman::{mmap, shm_open, shm_unlink, MapFlags, ProtFlags};
    use nix::sys::stat::Mode;

    let shm_name = "/vrift_bloom";
    let _ = shm_unlink(shm_name); // Cleanup old
    let shm_fd = shm_open(
        shm_name,
        OFlag::O_CREAT | OFlag::O_RDWR,
        Mode::from_bits_retain(0o666),
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
    // Populate with existing manifest
    for path in manifest.paths() {
        bloom.add(path);
    }

    // Initialize shared state
    let state = Arc::new(DaemonState {
        cas_index: Mutex::new(HashMap::new()),
        manifest: Mutex::new(manifest),
        manifest_path,
        bloom,
    });

    // Start background scan (Warm-up)
    let scan_state = state.clone();
    tokio::spawn(async move {
        tracing::info!("vriftd: Starting CAS warm-up scan...");
        if let Err(e) = scan_cas_root(&scan_state).await {
            tracing::error!("vriftd: CAS scan failed: {}", e);
        } else {
            let count = scan_state.cas_index.lock().await.len();
            tracing::info!("vriftd: CAS warm-up complete. Indexed {} blobs.", count);
        }
    });

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

    loop {
        let mut len_buf = [0u8; 4];
        if stream.read_exact(&mut len_buf).await.is_err() {
            return;
        }
        let len = u32::from_le_bytes(len_buf) as usize;

        // DoS protection: cap message size
        if len > MAX_IPC_SIZE {
            tracing::warn!("IPC message too large: {} bytes, rejecting", len);
            return;
        }

        let mut buf = vec![0u8; len];
        if stream.read_exact(&mut buf).await.is_err() {
            return;
        }

        let response = match bincode::deserialize::<VeloRequest>(&buf) {
            Ok(req) => handle_request(req, &state, peer_creds, daemon_uid).await,
            Err(e) => VeloResponse::Error(format!("Invalid request: {}", e)),
        };

        let resp_bytes = match bincode::serialize(&response) {
            Ok(b) => b,
            Err(e) => {
                eprintln!("Failed to serialize response: {}", e);
                return;
            }
        };

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
) -> VeloResponse {
    tracing::debug!("Received request: {:?}", req);
    match req {
        VeloRequest::Handshake { client_version } => {
            tracing::info!("Handshake from client: {}", client_version);
            VeloResponse::HandshakeAck {
                server_version: env!("CARGO_PKG_VERSION").to_string(),
            }
        }
        VeloRequest::Status => {
            let count = state.cas_index.lock().await.len();
            VeloResponse::StatusAck {
                status: format!("Operational (Indexed: {} blobs)", count),
            }
        }
        VeloRequest::Spawn { command, env, cwd } => {
            // Security: Only allow same-UID or root to spawn
            if let Some(creds) = peer_creds {
                if creds.uid != daemon_uid && creds.uid != 0 {
                    tracing::warn!(
                        "Spawn denied: peer UID {} != daemon UID {}",
                        creds.uid,
                        daemon_uid
                    );
                    return VeloResponse::Error("Permission denied: UID mismatch".to_string());
                }
            } else {
                return VeloResponse::Error(
                    "Permission denied: unable to verify peer credentials".to_string(),
                );
            }
            handle_spawn(command, env, cwd).await
        }
        VeloRequest::CasInsert { hash, size } => {
            let mut index = state.cas_index.lock().await;
            index.insert(hash, size);
            VeloResponse::CasAck
        }
        VeloRequest::CasGet { hash } => {
            let index = state.cas_index.lock().await;
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
        } => handle_protect(path, immutable, owner).await,
        VeloRequest::ManifestGet { path } => {
            let manifest: tokio::sync::MutexGuard<Manifest> = state.manifest.lock().await;
            VeloResponse::ManifestAck {
                entry: manifest.get(&path).cloned(),
            }
        }
        VeloRequest::ManifestUpsert { path, entry } => {
            let mut manifest: tokio::sync::MutexGuard<Manifest> = state.manifest.lock().await;
            manifest.insert(&path, entry);
            state.bloom.add(&path);
            if let Err(e) = manifest.save(&state.manifest_path) {
                tracing::error!("Failed to save manifest: {}", e);
            }
            VeloResponse::ManifestAck { entry: None }
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

async fn scan_cas_root(state: &DaemonState) -> Result<()> {
    // Get path from env or default
    let cas_root_str =
        std::env::var("VR_THE_SOURCE").unwrap_or_else(|_| "~/.vrift/the_source".to_string());
    let cas_root = Path::new(&cas_root_str);

    if !cas_root.exists() {
        println!(
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

    let mut index = state.cas_index.lock().await;

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
