//! Unix Domain Socket listener for vdir_d

use crate::commands::CommandHandler;
use crate::vdir::VDir;
use crate::ProjectConfig;
use anyhow::Result;
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::RwLock;
use tracing::{debug, error, info, warn};
use vrift_ipc::{VeloRequest, VeloResponse};

/// Run the UDS listener loop
pub async fn run_listener(config: ProjectConfig, vdir: VDir) -> Result<()> {
    // Remove existing socket if present
    if config.socket_path.exists() {
        std::fs::remove_file(&config.socket_path)?;
    }

    let listener = UnixListener::bind(&config.socket_path)?;
    info!(socket = %config.socket_path.display(), "Listening for connections");

    let handler = Arc::new(RwLock::new(CommandHandler::new(config.clone(), vdir)));

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

/// Handle a single client connection
async fn handle_client(mut stream: UnixStream, handler: Arc<RwLock<CommandHandler>>) -> Result<()> {
    debug!("New client connected");

    loop {
        // Read length prefix
        let mut len_buf = [0u8; 4];
        match stream.read_exact(&mut len_buf).await {
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
                debug!("Client disconnected");
                return Ok(());
            }
            Err(e) => return Err(e.into()),
        }

        let msg_len = u32::from_le_bytes(len_buf) as usize;
        if msg_len > 1024 * 1024 {
            warn!(len = msg_len, "Message too large, dropping client");
            return Ok(());
        }

        // Read payload
        let mut payload = vec![0u8; msg_len];
        stream.read_exact(&mut payload).await?;

        // Deserialize request
        let request: VeloRequest = match bincode::deserialize(&payload) {
            Ok(req) => req,
            Err(e) => {
                warn!(error = %e, "Failed to deserialize request");
                let response = VeloResponse::Error(format!("Deserialize error: {}", e));
                send_response(&mut stream, &response).await?;
                continue;
            }
        };

        debug!(?request, "Received request");

        // Handle request
        let response = {
            let mut h = handler.write().await;
            h.handle_request(request).await
        };

        // Send response
        send_response(&mut stream, &response).await?;
    }
}

/// Send response with length prefix
async fn send_response(stream: &mut UnixStream, response: &VeloResponse) -> Result<()> {
    let payload = bincode::serialize(response)?;
    let len = (payload.len() as u32).to_le_bytes();
    stream.write_all(&len).await?;
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
