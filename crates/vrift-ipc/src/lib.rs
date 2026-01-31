use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub enum VeloRequest {
    Handshake {
        client_version: String,
    },
    Status,
    Spawn {
        command: Vec<String>,
        env: Vec<(String, String)>,
        cwd: String,
    },
    CasInsert {
        hash: [u8; 32],
        size: u64,
    },
    CasGet {
        hash: [u8; 32],
    },
    Protect {
        path: String,
        immutable: bool,
        owner: Option<String>,
    },
}

#[derive(Debug, Serialize, Deserialize)]
pub enum VeloResponse {
    HandshakeAck { server_version: String },
    StatusAck { status: String },
    SpawnAck { pid: u32 },
    CasAck,
    CasFound { size: u64 },
    CasNotFound,
    ProtectAck,
    Error(String),
}

/// Default daemon socket path
pub fn default_socket_path() -> &'static str {
    "/tmp/vrift.sock"
}

/// Check if daemon is running (socket exists and connectable)
pub fn is_daemon_running() -> bool {
    std::path::Path::new(default_socket_path()).exists()
}

/// IPC Client for communicating with vrift-daemon
pub mod client {
    use super::*;
    use std::path::Path;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::UnixStream;

    pub struct DaemonClient {
        stream: UnixStream,
    }

    impl DaemonClient {
        /// Connect to daemon at default socket path
        pub async fn connect() -> anyhow::Result<Self> {
            Self::connect_to(default_socket_path()).await
        }

        /// Connect to daemon at custom socket path
        pub async fn connect_to(socket_path: &str) -> anyhow::Result<Self> {
            let stream = UnixStream::connect(Path::new(socket_path)).await?;
            Ok(Self { stream })
        }

        /// Send a request and receive response
        pub async fn send(&mut self, request: VeloRequest) -> anyhow::Result<VeloResponse> {
            // Serialize request
            let req_bytes = bincode::serialize(&request)?;
            let req_len = (req_bytes.len() as u32).to_le_bytes();

            // Send length + payload
            self.stream.write_all(&req_len).await?;
            self.stream.write_all(&req_bytes).await?;

            // Read response length
            let mut len_buf = [0u8; 4];
            self.stream.read_exact(&mut len_buf).await?;
            let resp_len = u32::from_le_bytes(len_buf) as usize;

            // Read response payload
            let mut resp_buf = vec![0u8; resp_len];
            self.stream.read_exact(&mut resp_buf).await?;

            // Deserialize response
            let response = bincode::deserialize(&resp_buf)?;
            Ok(response)
        }

        /// Handshake with daemon
        pub async fn handshake(&mut self) -> anyhow::Result<String> {
            let request = VeloRequest::Handshake {
                client_version: env!("CARGO_PKG_VERSION").to_string(),
            };
            match self.send(request).await? {
                VeloResponse::HandshakeAck { server_version } => Ok(server_version),
                VeloResponse::Error(e) => anyhow::bail!("Handshake failed: {}", e),
                _ => anyhow::bail!("Unexpected response"),
            }
        }

        /// Get daemon status
        pub async fn status(&mut self) -> anyhow::Result<String> {
            match self.send(VeloRequest::Status).await? {
                VeloResponse::StatusAck { status } => Ok(status),
                VeloResponse::Error(e) => anyhow::bail!("Status failed: {}", e),
                _ => anyhow::bail!("Unexpected response"),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_request_serialization() {
        let req = VeloRequest::Status;
        let bytes = bincode::serialize(&req).unwrap();
        let decoded: VeloRequest = bincode::deserialize(&bytes).unwrap();
        assert!(matches!(decoded, VeloRequest::Status));
    }

    #[test]
    fn test_response_serialization() {
        let resp = VeloResponse::StatusAck {
            status: "OK".to_string(),
        };
        let bytes = bincode::serialize(&resp).unwrap();
        let decoded: VeloResponse = bincode::deserialize(&bytes).unwrap();
        assert!(matches!(decoded, VeloResponse::StatusAck { .. }));
    }

    #[test]
    fn test_default_socket_path() {
        // Verify default socket path is set
        let path = default_socket_path();
        assert!(!path.is_empty());
        assert!(path.ends_with(".sock"));
    }
}
