use anyhow::{Context, Result};
use std::path::{Path, PathBuf};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;
use velo_ipc::{VeloRequest, VeloResponse};

const SOCKET_PATH: &str = "/tmp/velo.sock";

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
            println!("Daemon Connected. Server v{}", server_version);
        }
        _ => anyhow::bail!("Unexpected handshake response: {:?}", resp),
    }

    // Status
    let req = VeloRequest::Status;
    send_request(&mut stream, req).await?;
    let resp = read_response(&mut stream).await?;

    match resp {
        VeloResponse::StatusAck { status } => {
            println!("Daemon Status: {}", status);
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

    println!("Requesting daemon to spawn: {:?}", command);
    send_request(&mut stream, req).await?;
    
    let resp = read_response(&mut stream).await?;
    match resp {
        VeloResponse::SpawnAck { pid } => {
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

async fn connect() -> Result<UnixStream> {
    match UnixStream::connect(SOCKET_PATH).await {
        Ok(stream) => Ok(stream),
        Err(_) => {
            // Attempt to start daemon
            println!("Daemon not running. Attempting to start...");
            spawn_daemon()?;
            
            // Retry connection loop
            for _ in 0..10 {
                tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
                if let Ok(stream) = UnixStream::connect(SOCKET_PATH).await {
                     println!("Daemon started successfully.");
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
    
    // Look for velod or velo-daemon
    let candidate_names = if cfg!(target_os = "windows") {
        vec!["velod.exe", "velo-daemon.exe"]
    } else {
        vec!["velod", "velo-daemon"]
    };

    let daemon_bin = candidate_names.iter()
        .map(|name| bin_dir.join(name))
        .find(|path| path.exists())
        .context("Could not find velo-daemon binary")?;

    println!("Spawning daemon: {:?}", daemon_bin);

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
