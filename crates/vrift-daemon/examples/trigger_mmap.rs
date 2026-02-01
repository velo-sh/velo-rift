use std::time::{SystemTime, UNIX_EPOCH};
use vrift_ipc::client::DaemonClient;
use vrift_ipc::VeloRequest;
use vrift_manifest::VnodeEntry;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut client = DaemonClient::connect().await?;
    client.handshake().await?;

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();

    let entry = VnodeEntry::new_file(
        [0u8; 32], // dummy hash
        1024, now, 0o644,
    );

    println!("Sending 1,000 ManifestUpserts to daemon (including /vrift/subdir)...");

    // 1. Root files
    for i in 0..10 {
        let path = format!("/vrift/root_{}.txt", i);
        let req = VeloRequest::ManifestUpsert {
            path,
            entry: entry.clone(),
        };
        client.send(req).await?;
    }

    // 2. Subdirectory entry
    let dir_entry = VnodeEntry::new_directory(now, 0o755);
    client
        .send(VeloRequest::ManifestUpsert {
            path: "/vrift/subdir".to_string(),
            entry: dir_entry,
        })
        .await?;

    // 3. Subdirectory files
    for i in 0..10 {
        let path = format!("/vrift/subdir/file_{}.txt", i);
        let req = VeloRequest::ManifestUpsert {
            path,
            entry: entry.clone(),
        };
        client.send(req).await?;
    }

    println!("Done. Check daemon logs and /tmp/vrift-manifest.mmap");
    Ok(())
}
