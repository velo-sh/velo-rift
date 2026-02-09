use anyhow::{Context, Result};
use std::path::{Path, PathBuf};
use tokio::net::UnixStream;
use vrift_config::path::{normalize_nonexistent, normalize_or_original};
use vrift_ipc::{VeloRequest, VeloResponse, PROTOCOL_VERSION};

/// Phase 1.2: Connection state returned by connect_to_daemon.
/// Contains the vriftd stream plus vDird connection info from RegisterAck.
#[allow(dead_code)]
pub struct DaemonConnection {
    pub stream: UnixStream,
    pub vdird_socket: String,
    pub vdir_mmap_path: String,
}

fn get_socket_path() -> PathBuf {
    vrift_config::config().socket_path().to_path_buf()
}

pub async fn check_status(_project_root: &Path) -> Result<()> {
    let mut stream = tokio::time::timeout(std::time::Duration::from_secs(10), connect_simple())
        .await
        .map_err(|_| anyhow::anyhow!("Timed out connecting to daemon (10s)"))??;

    // Status request
    let req = VeloRequest::Status;
    send_request(&mut stream, req).await?;
    let resp = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        read_response(&mut stream),
    )
    .await
    .map_err(|_| anyhow::anyhow!("Timed out waiting for daemon status (5s)"))??;

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
    let conn = connect_to_daemon(project_root).await?;
    let mut stream = conn.stream;

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
        Ok(conn) => {
            let mut stream = conn.stream;
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
    if let Ok(conn) = connect_to_daemon(project_root).await {
        let mut stream = conn.stream;
        let req = VeloRequest::CasInsert { hash, size };
        let _ = send_request(&mut stream, req).await;
    }
    Ok(())
}

pub async fn connect_to_daemon(project_root: &Path) -> Result<DaemonConnection> {
    let socket_path = get_socket_path();
    let connect_fut = UnixStream::connect(&socket_path);
    let mut stream =
        match tokio::time::timeout(std::time::Duration::from_secs(5), connect_fut).await {
            Ok(Ok(s)) => s,
            _ => {
                tracing::info!("Daemon not running. Attempting to start...");
                spawn_daemon()?;
                let mut s = None;
                for _ in 0..10 {
                    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
                    if let Ok(conn) = UnixStream::connect(&socket_path).await {
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
        protocol_version: PROTOCOL_VERSION,
    };
    send_request(&mut stream, handshake).await?;
    let _ = read_response(&mut stream).await?;

    // 2. Register Workspace (normalize to absolute path for daemon)
    let abs_project_root = normalize_or_original(project_root);
    let register = VeloRequest::RegisterWorkspace {
        project_root: abs_project_root.to_string_lossy().to_string(),
    };
    send_request(&mut stream, register).await?;
    let resp = read_response(&mut stream).await?;

    match resp {
        VeloResponse::RegisterAck {
            workspace_id: _,
            vdird_socket,
            vdir_mmap_path,
        } => Ok(DaemonConnection {
            stream,
            vdird_socket,
            vdir_mmap_path,
        }),
        VeloResponse::Error(e) => anyhow::bail!("Workspace registration failed: {}", e),
        _ => anyhow::bail!("Unexpected registration response"),
    }
}

/// Simple connection to daemon - only handshake, no workspace registration
/// Used for standalone operations like IngestFullScan
async fn connect_simple() -> Result<UnixStream> {
    let socket_path = get_socket_path();

    // Try connecting + handshake directly first
    if let Ok(mut stream) = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        UnixStream::connect(&socket_path),
    )
    .await
    .unwrap_or(Err(std::io::Error::new(
        std::io::ErrorKind::TimedOut,
        "timeout",
    ))) {
        let handshake = VeloRequest::Handshake {
            client_version: env!("CARGO_PKG_VERSION").to_string(),
            protocol_version: PROTOCOL_VERSION,
        };
        if send_request(&mut stream, handshake).await.is_ok() {
            if let Ok(resp) = read_response(&mut stream).await {
                match resp {
                    VeloResponse::HandshakeAck { .. } => return Ok(stream),
                    VeloResponse::Error(e) => anyhow::bail!("Handshake failed: {}", e),
                    _ => {}
                }
            }
        }
    }

    // Direct connect failed or handshake failed — spawn daemon and retry
    tracing::info!("Daemon not running. Attempting to start...");
    spawn_daemon()?;

    for attempt in 0..20 {
        tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;

        if let Ok(mut stream) = UnixStream::connect(&socket_path).await {
            let handshake = VeloRequest::Handshake {
                client_version: env!("CARGO_PKG_VERSION").to_string(),
                protocol_version: PROTOCOL_VERSION,
            };
            if send_request(&mut stream, handshake).await.is_ok() {
                if let Ok(resp) = read_response(&mut stream).await {
                    match resp {
                        VeloResponse::HandshakeAck { .. } => {
                            tracing::info!("Connected to daemon after {} attempts", attempt + 1);
                            return Ok(stream);
                        }
                        VeloResponse::Error(e) => anyhow::bail!("Handshake failed: {}", e),
                        _ => tracing::debug!("Unexpected handshake response, retrying..."),
                    }
                }
            }
        }
    }

    anyhow::bail!("Failed to connect to daemon after starting it (20 attempts)")
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

pub async fn send_request(stream: &mut UnixStream, req: VeloRequest) -> Result<u32> {
    tracing::debug!("Sending request: {:?}", req);
    let seq_id = vrift_ipc::frame_async::send_request(stream, &req).await?;
    Ok(seq_id)
}

pub async fn read_response(stream: &mut UnixStream) -> Result<VeloResponse> {
    tracing::debug!("[CLI] Waiting for response...");
    let (header, resp) = vrift_ipc::frame_async::read_response(stream).await?;
    tracing::debug!("[CLI] Response received, seq_id={}", header.seq_id);
    Ok(resp)
}

/// Ingest files via daemon (unified architecture)
/// CLI becomes thin client, daemon handles all ingest logic
/// Note: IngestFullScan is a standalone operation that doesn't require workspace registration
#[allow(clippy::too_many_arguments)]
pub async fn ingest_via_daemon(
    path: &Path,
    manifest_path: &Path,
    threads: Option<usize>,
    phantom: bool,
    tier1: bool,
    prefix: Option<String>,
    cas_root: Option<&Path>,
    force_hash: bool,
) -> Result<IngestResult> {
    // Normalize paths before sending to daemon (daemon's cwd may differ)
    let abs_path = normalize_or_original(path);
    // Manifest may not exist yet, so normalize parent + append filename
    let abs_manifest =
        normalize_nonexistent(manifest_path).unwrap_or_else(|_| manifest_path.to_path_buf());

    // Use simple connection - IngestFullScan doesn't need workspace context
    tracing::info!("[CLI] Connecting to daemon for ingest...");
    let mut stream = connect_simple().await?;
    tracing::info!("[CLI] Connected to daemon successfully");

    let req = VeloRequest::IngestFullScan {
        path: abs_path.to_string_lossy().to_string(),
        manifest_path: abs_manifest.to_string_lossy().to_string(),
        threads,
        phantom,
        tier1,
        prefix,
        cas_root: cas_root.map(|p| p.to_string_lossy().to_string()),
        force_hash,
    };

    tracing::info!(
        "[CLI] Sending IngestFullScan request: path={:?}, manifest={:?}",
        abs_path,
        abs_manifest
    );
    send_request(&mut stream, req).await?;
    tracing::info!("[CLI] IngestFullScan request sent, waiting for response...");

    // Ingest can take minutes for large datasets, use generous timeout
    let resp = tokio::time::timeout(
        std::time::Duration::from_secs(120),
        read_response(&mut stream),
    )
    .await
    .map_err(|_| anyhow::anyhow!("Timed out waiting for ingest response (120s)"))??;
    tracing::info!("[CLI] Received ingest response");
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
