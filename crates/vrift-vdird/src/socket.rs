//! Unix Domain Socket listener for vdir_d
//!
//! Uses IpcHeader frame protocol for all IPC communication.

use crate::commands::CommandHandler;
use crate::vdir::VDir;
use crate::ProjectConfig;
use anyhow::Result;
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::RwLock;
use tracing::{debug, error, info, warn};
use vrift_ipc::{IpcHeader, VeloError, VeloRequest, VeloResponse};

/// Run the UDS listener loop
pub async fn run_listener(
    config: ProjectConfig,
    vdir: VDir,
    manifest: std::sync::Arc<vrift_manifest::lmdb::LmdbManifest>,
) -> Result<()> {
    // Remove existing socket if present
    if config.socket_path.exists() {
        std::fs::remove_file(&config.socket_path)?;
    }

    let listener = UnixListener::bind(&config.socket_path)?;
    info!(socket = %config.socket_path.display(), "Listening for connections");

    let handler = Arc::new(RwLock::new(CommandHandler::new(
        config.clone(),
        vdir,
        manifest,
    )));

    loop {
        match listener.accept().await {
            Ok((stream, _addr)) => {
                let handler = Arc::clone(&handler);
                tokio::spawn(async move {
                    if let Err(e) = handle_client(stream, handler).await {
                        warn!(error = %e, "Client handler error");
                    }
                });
            }
            Err(e) => {
                error!(error = %e, "Accept failed");
            }
        }
    }
}

/// Handle a single client connection using IpcHeader frame protocol
async fn handle_client(mut stream: UnixStream, handler: Arc<RwLock<CommandHandler>>) -> Result<()> {
    debug!("New client connected");

    loop {
        // Read IpcHeader (12 bytes)
        let mut header_buf = [0u8; IpcHeader::SIZE];
        match stream.read_exact(&mut header_buf).await {
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
                debug!("Client disconnected");
                return Ok(());
            }
            Err(e) => return Err(e.into()),
        }

        let header = IpcHeader::from_bytes(&header_buf);
        if !header.is_valid() {
            if header.magic != vrift_ipc::IPC_MAGIC {
                warn!("Invalid IPC magic, dropping client");
            } else if header.version() != vrift_ipc::PROTOCOL_VERSION as u8 {
                warn!(
                    expected = vrift_ipc::PROTOCOL_VERSION,
                    got = header.version(),
                    "IPC protocol version mismatch, dropping client"
                );
            } else {
                warn!("Invalid IPC frame type, dropping client");
            }
            return Ok(());
        }

        // RFC-0053: Handle heartbeats - log and skip
        if header.frame_type() == Some(vrift_ipc::FrameType::Heartbeat) {
            debug!(seq_id = header.seq_id, "Received heartbeat");
            continue;
        }

        // Ensure it's a request
        if header.frame_type() != Some(vrift_ipc::FrameType::Request) {
            warn!(
                ?header,
                "Expected request frame, but got instead. Dropping client."
            );
            return Ok(());
        }

        if header.length as usize > IpcHeader::MAX_LENGTH {
            warn!(len = header.length, "Message too large, dropping client");
            return Ok(());
        }

        // Read payload
        let mut payload = vec![0u8; header.length as usize];
        if !payload.is_empty() {
            stream.read_exact(&mut payload).await?;
        }

        // Deserialize request
        let request: VeloRequest =
            match rkyv::from_bytes::<VeloRequest, rkyv::rancor::Error>(&payload) {
                Ok(req) => req,
                Err(e) => {
                    warn!(error = %e, "Failed to deserialize request");
                    let response = VeloResponse::Error(VeloError::internal(format!(
                        "Deserialize error: {}",
                        e
                    )));
                    send_response(&mut stream, &response, header.seq_id).await?;
                    continue;
                }
            };

        debug!(?request, "Received request");

        // Handle request
        let response = {
            let mut h = handler.write().await;
            h.handle_request(request).await
        };

        // Send response with matching seq_id
        send_response(&mut stream, &response, header.seq_id).await?;
    }
}

/// Send response using IpcHeader frame protocol
async fn send_response(
    stream: &mut UnixStream,
    response: &VeloResponse,
    seq_id: u32,
) -> Result<()> {
    let payload = rkyv::to_bytes::<rkyv::rancor::Error>(response)
        .map_err(|e| anyhow::anyhow!("Serialize error: {}", e))?;

    if payload.len() > IpcHeader::MAX_LENGTH {
        return Err(anyhow::anyhow!(
            "Response too large: {} bytes",
            payload.len()
        ));
    }

    let header = IpcHeader::new_response(payload.len() as u32, seq_id);

    stream.write_all(&header.to_bytes()).await?;
    stream.write_all(&payload).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[tokio::test]
    async fn test_socket_accepts_connection() {
        let temp = tempdir().unwrap();
        let socket_path = temp.path().join("test.sock");

        let listener = UnixListener::bind(&socket_path).unwrap();

        // Connect in background
        let socket_path_clone = socket_path.clone();
        let handle =
            tokio::spawn(async move { tokio::net::UnixStream::connect(&socket_path_clone).await });

        // Accept connection
        let result =
            tokio::time::timeout(std::time::Duration::from_millis(100), listener.accept()).await;

        assert!(result.is_ok());
        handle.await.unwrap().unwrap();
    }
}
