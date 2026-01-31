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
}

#[derive(Debug, Serialize, Deserialize)]
pub enum VeloResponse {
    HandshakeAck { server_version: String },
    StatusAck { status: String },
    SpawnAck { pid: u32 },
    CasAck,
    CasFound { size: u64 },
    CasNotFound,
    Error(String),
}
