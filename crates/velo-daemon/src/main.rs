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
    println!("velod: Starting daemon...");

    let socket_path = "/tmp/velo.sock";
    let path = Path::new(socket_path);

    if path.exists() {
        tokio::fs::remove_file(path).await?;
    }

    let listener = UnixListener::bind(path)?;
    println!("velod: Listening on {}", socket_path);

    // Initialize shared state
    let state = Arc::new(DaemonState {
        cas_index: Mutex::new(HashMap::new()),
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
                        eprintln!("velod: Accept error: {}", err);
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
    println!("Received request: {:?}", req);
    match req {
        VeloRequest::Handshake { client_version } => {
            println!("Handshake from client: {}", client_version);
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
