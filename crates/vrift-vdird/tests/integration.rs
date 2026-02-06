//! Integration tests for vrift-vdird
//!
//! These tests verify complete daemon lifecycle and client-server communication.

use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::time::Duration;
use tempfile::tempdir;

/// Helper to send request and receive response using bincode
fn send_request(
    stream: &mut UnixStream,
    request: &vrift_ipc::VeloRequest,
) -> vrift_ipc::VeloResponse {
    // Ensure blocking mode for reliable read
    stream.set_nonblocking(false).ok();

    let payload = bincode::serialize(request).unwrap();
    let len = (payload.len() as u32).to_le_bytes();
    stream.write_all(&len).unwrap();
    stream.write_all(&payload).unwrap();

    let mut len_buf = [0u8; 4];
    stream.read_exact(&mut len_buf).unwrap();
    let resp_len = u32::from_le_bytes(len_buf) as usize;

    let mut resp_buf = vec![0u8; resp_len];
    stream.read_exact(&mut resp_buf).unwrap();

    bincode::deserialize(&resp_buf).unwrap()
}

#[tokio::test]
#[ignore] // Requires manual daemon lifecycle - run with --ignored
async fn test_daemon_lifecycle_handshake() {
    let temp = tempdir().unwrap();
    let socket_path = temp.path().join("test.sock");

    // Start daemon in background
    let config = vrift_vdird::ProjectConfig {
        project_root: temp.path().to_path_buf(),
        project_id: "test_project".to_string(),
        vdir_path: temp.path().join("test.vdir"),
        socket_path: socket_path.clone(),
        staging_base: temp.path().join("staging"),
        cas_path: temp.path().join("the_source"),
    };

    // Create required directories
    std::fs::create_dir_all(&config.staging_base).unwrap();
    std::fs::create_dir_all(&config.cas_path).unwrap();

    let config_clone = config.clone();
    let daemon_handle = tokio::spawn(async move { vrift_vdird::run_daemon(config_clone).await });

    // Wait for socket to be ready
    for _ in 0..50 {
        if socket_path.exists() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }

    // Connect and send handshake
    let mut stream = UnixStream::connect(&socket_path).unwrap();

    let response = send_request(
        &mut stream,
        &vrift_ipc::VeloRequest::Handshake {
            client_version: "1.0.0".to_string(),
            protocol_version: vrift_ipc::PROTOCOL_VERSION,
        },
    );

    match response {
        vrift_ipc::VeloResponse::HandshakeAck { server_version, .. } => {
            assert!(!server_version.is_empty());
        }
        _ => panic!("Expected HandshakeAck"),
    }

    // Cleanup
    daemon_handle.abort();
}

#[tokio::test]
#[ignore] // Requires manual daemon lifecycle - run with --ignored
async fn test_daemon_manifest_operations_over_socket() {
    let temp = tempdir().unwrap();
    let socket_path = temp.path().join("test.sock");

    let config = vrift_vdird::ProjectConfig {
        project_root: temp.path().to_path_buf(),
        project_id: "test_project".to_string(),
        vdir_path: temp.path().join("test.vdir"),
        socket_path: socket_path.clone(),
        staging_base: temp.path().join("staging"),
        cas_path: temp.path().join("the_source"),
    };

    std::fs::create_dir_all(&config.staging_base).unwrap();
    std::fs::create_dir_all(&config.cas_path).unwrap();

    let config_clone = config.clone();
    let daemon_handle = tokio::spawn(async move { vrift_vdird::run_daemon(config_clone).await });

    // Wait for socket
    for _ in 0..50 {
        if socket_path.exists() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }

    let mut stream = UnixStream::connect(&socket_path).unwrap();

    // Upsert
    let entry = vrift_ipc::VnodeEntry {
        content_hash: [42; 32],
        size: 1000,
        mtime: 1234567890,
        mode: 0o644,
        flags: 0,
        _pad: 0,
    };

    let response = send_request(
        &mut stream,
        &vrift_ipc::VeloRequest::ManifestUpsert {
            path: "src/main.rs".to_string(),
            entry,
        },
    );
    assert!(matches!(
        response,
        vrift_ipc::VeloResponse::ManifestAck { .. }
    ));

    // Get
    let response = send_request(
        &mut stream,
        &vrift_ipc::VeloRequest::ManifestGet {
            path: "src/main.rs".to_string(),
        },
    );

    match response {
        vrift_ipc::VeloResponse::ManifestAck { entry: Some(e) } => {
            assert_eq!(e.size, 1000);
            assert_eq!(e.content_hash, [42; 32]);
        }
        _ => panic!("Expected entry"),
    }

    daemon_handle.abort();
}

#[tokio::test]
#[ignore] // Requires manual daemon lifecycle - run with --ignored
async fn test_daemon_multiple_clients() {
    let temp = tempdir().unwrap();
    let socket_path = temp.path().join("test.sock");

    let config = vrift_vdird::ProjectConfig {
        project_root: temp.path().to_path_buf(),
        project_id: "test_project".to_string(),
        vdir_path: temp.path().join("test.vdir"),
        socket_path: socket_path.clone(),
        staging_base: temp.path().join("staging"),
        cas_path: temp.path().join("the_source"),
    };

    std::fs::create_dir_all(&config.staging_base).unwrap();
    std::fs::create_dir_all(&config.cas_path).unwrap();

    let config_clone = config.clone();
    let daemon_handle = tokio::spawn(async move { vrift_vdird::run_daemon(config_clone).await });

    // Wait for socket
    for _ in 0..50 {
        if socket_path.exists() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }

    // Connect multiple clients
    let mut streams: Vec<UnixStream> = Vec::new();
    for _ in 0..3 {
        let stream = UnixStream::connect(&socket_path).unwrap();
        streams.push(stream);
    }

    // Each client sends handshake
    for stream in &mut streams {
        let response = send_request(
            stream,
            &vrift_ipc::VeloRequest::Handshake {
                client_version: "1.0.0".to_string(),
                protocol_version: vrift_ipc::PROTOCOL_VERSION,
            },
        );
        assert!(matches!(
            response,
            vrift_ipc::VeloResponse::HandshakeAck { .. }
        ));
    }

    daemon_handle.abort();
}
