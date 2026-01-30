use anyhow::Result;
use clap::{Parser, Subcommand};

use tokio::signal;

#[derive(Parser)]
#[command(name = "velod")]
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
use tokio::net::{UnixListener, UnixStream};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use velo_ipc::{VeloRequest, VeloResponse};

use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;

struct DaemonState {
    // In-memory index of CAS blobs (Hash -> Size)
    cas_index: Mutex<HashMap<[u8; 32], u64>>,
}

async fn start_daemon() -> Result<()> {
    tracing::info!("velod: Starting daemon...");

    let socket_path = "/tmp/velo.sock";
    let path = Path::new(socket_path);

    if path.exists() {
        tokio::fs::remove_file(path).await?;
    }

    let listener = UnixListener::bind(path)?;
    tracing::info!("velod: Listening on {}", socket_path);

    // Initialize shared state
    let state = Arc::new(DaemonState {
        cas_index: Mutex::new(HashMap::new()),
    });

    // Start background scan (Warm-up)
    let scan_state = state.clone();
    tokio::spawn(async move {
        tracing::info!("velod: Starting CAS warm-up scan...");
        if let Err(e) = scan_cas_root(&scan_state).await {
            tracing::error!("velod: CAS scan failed: {}", e);
        } else {
            let count = scan_state.cas_index.lock().await.len();
            tracing::info!("velod: CAS warm-up complete. Indexed {} blobs.", count);
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
                        tracing::error!("velod: Accept error: {}", err);
                    }
                }
            }
            _ = signal::ctrl_c() => {
                println!("velod: Shutdown signal received");
                break;
            }
        }
    }

    println!("velod: Shutting down");
    if path.exists() {
        tokio::fs::remove_file(path).await?;
    }

    Ok(())
}

async fn handle_connection(mut stream: UnixStream, state: Arc<DaemonState>) {
    loop {
        let mut len_buf = [0u8; 4];
        if let Err(_) = stream.read_exact(&mut len_buf).await {
            return;
        }
        let len = u32::from_le_bytes(len_buf) as usize;

        let mut buf = vec![0u8; len];
        if let Err(_) = stream.read_exact(&mut buf).await {
            return;
        }

        let response = match bincode::deserialize::<VeloRequest>(&buf) {
            Ok(req) => handle_request(req, &state).await,
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
        if let Err(_) = stream.write_all(&resp_len).await {
            return;
        }
        if let Err(_) = stream.write_all(&resp_bytes).await {
            return;
        }
    }
}

async fn handle_request(req: VeloRequest, state: &DaemonState) -> VeloResponse {
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
    }
}

async fn handle_spawn(command: Vec<String>, env: Vec<(String, String)>, cwd: String) -> VeloResponse {
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
        Err(e) => VeloResponse::Error(format!("Failed to spawn: {}", e))
    }
}

async fn scan_cas_root(state: &DaemonState) -> Result<()> {
    // Get path from env or default
    let cas_root_str = std::env::var("VELO_CAS_ROOT").unwrap_or_else(|_| "/var/velo/the_source".to_string());
    let cas_root = Path::new(&cas_root_str);
    
    if !cas_root.exists() {
        println!("velod: CAS root not found at {:?}, skipping scan.", cas_root);
        return Ok(());
    }

    use velo_cas::CasStore;
    let cas = CasStore::new(cas_root)?;
    
    // We can use CasStore's iterator, but it's synchronous (blocking).
    // For now, we'll wrap it in spawn_blocking or just run it since we are in a dedicated task.
    // Iterating millions of files might take time, so blocking the runtime is bad if not careful.
    // But this is a separate task.
    
    let mut index = state.cas_index.lock().await;
    
    // Using blocking iterator
    for hash_res in cas.iter()? {
        if let Ok(hash) = hash_res {
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
    }
    
    Ok(())
}
