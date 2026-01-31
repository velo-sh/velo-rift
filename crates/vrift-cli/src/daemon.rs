use anyhow::{Context, Result};
use std::path::PathBuf;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;
use vrift_ipc::{VeloRequest, VeloResponse};

const SOCKET_PATH: &str = "/tmp/vrift.sock";

pub async fn check_status() -> Result<()> {
    let mut stream = connect().await?;

    // Handshake
    let req = VeloRequest::Handshake {
        client_version: env!("CARGO_PKG_VERSION").to_string(),
    };
    send_request(&mut stream, req).await?;
    let resp = read_response(&mut stream).await?;

    match resp {
        VeloResponse::HandshakeAck { server_version } => {
            tracing::info!("Daemon Connected. Server v{}", server_version);
        }
        _ => anyhow::bail!("Unexpected handshake response: {:?}", resp),
    }

    // Status
    let req = VeloRequest::Status;
    send_request(&mut stream, req).await?;
    let resp = read_response(&mut stream).await?;

    match resp {
        VeloResponse::StatusAck { status } => {
            println!("Daemon Status: {}", status); // Keep println for user output
            tracing::debug!("Daemon Status Details: {}", status);
        }
        _ => anyhow::bail!("Unexpected status response: {:?}", resp),
    }

    Ok(())
}

pub async fn spawn_command(command: &[String], cwd: PathBuf) -> Result<()> {
    let mut stream = connect().await?;

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
            println!("Daemon successfully spawned process. PID: {}", pid); // Keep for user
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
pub async fn check_blob(hash: [u8; 32]) -> Result<bool> {
    match connect().await {
        Ok(mut stream) => {
            let req = VeloRequest::CasGet { hash };
            if send_request(&mut stream, req).await.is_err() {
                return Ok(false); // Connection broke during send
            }
            match read_response(&mut stream).await {
                Ok(VeloResponse::CasFound { .. }) => Ok(true),
                Ok(VeloResponse::CasNotFound) => Ok(false),
                _ => Ok(false),
            }
        }
        Err(_) => Ok(false), // Daemon offline, assume not in memory
    }
}

#[allow(dead_code)]
pub async fn notify_blob(hash: [u8; 32], size: u64) -> Result<()> {
    // Fire and forget (optional) or wait for ack
    if let Ok(mut stream) = connect().await {
        let req = VeloRequest::CasInsert { hash, size };
        let _ = send_request(&mut stream, req).await;
        // Ideally wait for Ack, but for speed maybe we don't care if it fails
        // let _ = read_response(&mut stream).await;
    }
    Ok(())
}

pub async fn protect_file(
    path: std::path::PathBuf,
    immutable: bool,
    owner: Option<String>,
) -> Result<()> {
    match connect().await {
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
            // Non-critical if daemon not running (just less protection)
            Ok(())
        }
    }
}

async fn connect() -> Result<UnixStream> {
    match UnixStream::connect(SOCKET_PATH).await {
        Ok(stream) => Ok(stream),
        Err(_) => {
            // Attempt to start daemon
            tracing::info!("Daemon not running. Attempting to start...");
            spawn_daemon()?;

            // Retry connection loop
            for _ in 0..10 {
                tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
                if let Ok(stream) = UnixStream::connect(SOCKET_PATH).await {
                    tracing::info!("Daemon started successfully.");
                    return Ok(stream);
                }
            }
            anyhow::bail!("Failed to connect to daemon after starting it.");
        }
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

async fn send_request(stream: &mut UnixStream, req: VeloRequest) -> Result<()> {
    tracing::debug!("Sending request: {:?}", req);
    let bytes = bincode::serialize(&req)?;
    let len = (bytes.len() as u32).to_le_bytes();
    stream.write_all(&len).await?;
    stream.write_all(&bytes).await?;
    Ok(())
}

async fn read_response(stream: &mut UnixStream) -> Result<VeloResponse> {
    let mut len_buf = [0u8; 4];
    stream.read_exact(&mut len_buf).await?;
    let len = u32::from_le_bytes(len_buf) as usize;

    let mut buf = vec![0u8; len];
    stream.read_exact(&mut buf).await?;

    let resp = bincode::deserialize(&buf)?;
    Ok(resp)
}
