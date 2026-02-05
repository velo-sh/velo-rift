use anyhow::{Context, Result};
use std::path::{Path, PathBuf};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;
use vrift_ipc::{VeloRequest, VeloResponse};

const SOCKET_PATH: &str = "/tmp/vrift.sock";

pub async fn check_status(project_root: &Path) -> Result<()> {
    let mut stream = connect_to_daemon(project_root).await?;

    // Status request
    let req = VeloRequest::Status;
    send_request(&mut stream, req).await?;
    let resp = read_response(&mut stream).await?;

    match resp {
        VeloResponse::StatusAck { status } => {
            println!("Daemon Status: {}", status);
        }
        VeloResponse::Error(e) => anyhow::bail!("Status failed: {}", e),
        _ => anyhow::bail!("Unexpected status response: {:?}", resp),
    }

    Ok(())
}

pub async fn spawn_command(command: &[String], cwd: PathBuf, project_root: &Path) -> Result<()> {
    let mut stream = connect_to_daemon(project_root).await?;

    // Construct environment with explicit Strings
    let env: Vec<(String, String)> = std::env::vars().collect();

    let req = VeloRequest::Spawn {
        command: command.to_vec(),
        env,
        cwd: cwd.to_string_lossy().to_string(),
    };

    tracing::info!("Requesting daemon to spawn: {:?}", command);
    send_request(&mut stream, req).await?;

    let resp = read_response(&mut stream).await?;
    match resp {
        VeloResponse::SpawnAck { pid } => {
            tracing::info!("Daemon successfully spawned process. PID: {}", pid);
            println!("Daemon successfully spawned process. PID: {}", pid);
            println!("(Output will be in daemon logs for now)");
        }
        VeloResponse::Error(msg) => {
            anyhow::bail!("Daemon refused to spawn: {}", msg);
        }
        _ => anyhow::bail!("Unexpected response from daemon: {:?}", resp),
    }

    Ok(())
}

#[allow(dead_code)]
pub async fn check_blob(hash: [u8; 32], project_root: &Path) -> Result<bool> {
    match connect_to_daemon(project_root).await {
        Ok(mut stream) => {
            let req = VeloRequest::CasGet { hash };
            send_request(&mut stream, req).await?;
            match read_response(&mut stream).await? {
                VeloResponse::CasFound { .. } => Ok(true),
                VeloResponse::CasNotFound => Ok(false),
                VeloResponse::Error(e) => anyhow::bail!("Check failed: {}", e),
                _ => anyhow::bail!("Unexpected response"),
            }
        }
        Err(_) => Ok(false),
    }
}

#[allow(dead_code)]
pub async fn notify_blob(hash: [u8; 32], size: u64, project_root: &Path) -> Result<()> {
    if let Ok(mut stream) = connect_to_daemon(project_root).await {
        let req = VeloRequest::CasInsert { hash, size };
        let _ = send_request(&mut stream, req).await;
    }
    Ok(())
}

pub async fn protect_file(
    path: std::path::PathBuf,
    immutable: bool,
    owner: Option<String>,
    project_root: &Path,
) -> Result<()> {
    match connect_to_daemon(project_root).await {
        Ok(mut stream) => {
            let req = VeloRequest::Protect {
                path: path.to_string_lossy().to_string(),
                immutable,
                owner,
            };
            send_request(&mut stream, req).await?;
            match read_response(&mut stream).await? {
                VeloResponse::ProtectAck => Ok(()),
                VeloResponse::Error(e) => anyhow::bail!("Daemon protection failed: {}", e),
                _ => anyhow::bail!("Unexpected response from daemon"),
            }
        }
        Err(e) => {
            tracing::warn!("Daemon not available for protection: {}", e);
            Ok(())
        }
    }
}

pub async fn connect_to_daemon(project_root: &Path) -> Result<UnixStream> {
    let mut stream = match UnixStream::connect(SOCKET_PATH).await {
        Ok(s) => s,
        Err(_) => {
            tracing::info!("Daemon not running. Attempting to start...");
            spawn_daemon()?;
            let mut s = None;
            for _ in 0..10 {
                tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
                if let Ok(conn) = UnixStream::connect(SOCKET_PATH).await {
                    s = Some(conn);
                    break;
                }
            }
            s.context("Failed to connect to daemon after starting it")?
        }
    };

    // 1. Handshake
    let handshake = VeloRequest::Handshake {
        client_version: env!("CARGO_PKG_VERSION").to_string(),
    };
    send_request(&mut stream, handshake).await?;
    let _ = read_response(&mut stream).await?;

    // 2. Register Workspace
    let register = VeloRequest::RegisterWorkspace {
        project_root: project_root.to_string_lossy().to_string(),
    };
    send_request(&mut stream, register).await?;
    let resp = read_response(&mut stream).await?;

    match resp {
        VeloResponse::RegisterAck { .. } => Ok(stream),
        VeloResponse::Error(e) => anyhow::bail!("Workspace registration failed: {}", e),
        _ => anyhow::bail!("Unexpected registration response"),
    }
}

/// Simple connection to daemon - only handshake, no workspace registration
/// Used for standalone operations like IngestFullScan
async fn connect_simple() -> Result<UnixStream> {
    let mut stream = match UnixStream::connect(SOCKET_PATH).await {
        Ok(s) => s,
        Err(_) => {
            tracing::info!("Daemon not running. Attempting to start...");
            spawn_daemon()?;
            let mut s = None;
            for _ in 0..10 {
                tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
                if let Ok(conn) = UnixStream::connect(SOCKET_PATH).await {
                    s = Some(conn);
                    break;
                }
            }
            s.context("Failed to connect to daemon after starting it")?
        }
    };

    // Only handshake - no workspace registration
    let handshake = VeloRequest::Handshake {
        client_version: env!("CARGO_PKG_VERSION").to_string(),
    };
    send_request(&mut stream, handshake).await?;
    let resp = read_response(&mut stream).await?;

    match resp {
        VeloResponse::HandshakeAck { .. } => Ok(stream),
        VeloResponse::Error(e) => anyhow::bail!("Handshake failed: {}", e),
        _ => anyhow::bail!("Unexpected handshake response"),
    }
}

fn spawn_daemon() -> Result<()> {
    let current_exe = std::env::current_exe()?;
    let bin_dir = current_exe.parent().context("Failed into get bin dir")?;

    // Look for vriftd
    let candidate_names = if cfg!(target_os = "windows") {
        vec!["vriftd.exe"]
    } else {
        vec!["vriftd"]
    };

    let daemon_bin = candidate_names
        .iter()
        .map(|name| bin_dir.join(name))
        .find(|path| path.exists())
        .context("Could not find vriftd binary")?;

    tracing::info!("Spawning daemon: {:?}", daemon_bin);

    std::process::Command::new(daemon_bin)
        .arg("start")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .context("Failed to spawn daemon process")?;

    Ok(())
}

pub async fn send_request(stream: &mut UnixStream, req: VeloRequest) -> Result<()> {
    tracing::debug!("Sending request: {:?}", req);
    let bytes = bincode::serialize(&req)?;
    let len = (bytes.len() as u32).to_le_bytes();
    stream.write_all(&len).await?;
    stream.write_all(&bytes).await?;
    Ok(())
}

pub async fn read_response(stream: &mut UnixStream) -> Result<VeloResponse> {
    let mut len_buf = [0u8; 4];
    stream.read_exact(&mut len_buf).await?;
    let len = u32::from_le_bytes(len_buf) as usize;

    let mut buf = vec![0u8; len];
    stream.read_exact(&mut buf).await?;

    let resp = bincode::deserialize(&buf)?;
    Ok(resp)
}

/// Ingest files via daemon (unified architecture)
/// CLI becomes thin client, daemon handles all ingest logic
/// Note: IngestFullScan is a standalone operation that doesn't require workspace registration
pub async fn ingest_via_daemon(
    path: &Path,
    manifest_path: &Path,
    threads: Option<usize>,
    phantom: bool,
    tier1: bool,
) -> Result<IngestResult> {
    // Use simple connection - IngestFullScan doesn't need workspace context
    let mut stream = connect_simple().await?;

    let req = VeloRequest::IngestFullScan {
        path: path.to_string_lossy().to_string(),
        manifest_path: manifest_path.to_string_lossy().to_string(),
        threads,
        phantom,
        tier1,
    };

    tracing::info!("Requesting daemon to ingest: {:?}", path);
    send_request(&mut stream, req).await?;

    let resp = read_response(&mut stream).await?;
    match resp {
        VeloResponse::IngestAck {
            files,
            blobs,
            new_bytes,
            total_bytes,
            duration_ms,
            manifest_path,
        } => Ok(IngestResult {
            files,
            blobs,
            new_bytes,
            total_bytes,
            duration_ms,
            manifest_path,
        }),
        VeloResponse::Error(e) => anyhow::bail!("Daemon ingest failed: {}", e),
        _ => anyhow::bail!("Unexpected response from daemon: {:?}", resp),
    }
}

/// Result from daemon ingest
#[derive(Debug)]
#[allow(dead_code)]
pub struct IngestResult {
    pub files: u64,
    pub blobs: u64,
    pub new_bytes: u64,
    pub total_bytes: u64,
    pub duration_ms: u64,
    pub manifest_path: String,
}
