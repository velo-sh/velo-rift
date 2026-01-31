use std::fs::{self, File};
use std::io::Write;

use std::time::Instant;
use tempfile::TempDir;
use vrift_cas::CasStore;

#[test]
fn stress_test_mass_ingest() {
    // Quick stress test for CI (100 files ~1 sec)
    const FILE_COUNT: usize = 100;

    let temp = TempDir::new().unwrap();
    let src_dir = temp.path().join("src");
    let cas_dir = temp.path().join("cas");

    fs::create_dir(&src_dir).unwrap();

    println!("Generating {} files...", FILE_COUNT);
    let start_gen = Instant::now();

    for i in 0..FILE_COUNT {
        let p = src_dir.join(format!("file_{}.txt", i));
        let mut f = File::create(p).unwrap();
        // Alternating content to test dedup (50% unique)
        let content = if i % 2 == 0 {
            format!("content unique {}", i)
        } else {
            "shared content".to_string()
        };
        writeln!(f, "{}", content).unwrap();
    }

    println!("Generation took: {:?}", start_gen.elapsed());

    let cas = CasStore::new(&cas_dir).unwrap();

    println!("Ingesting...");
    let start_ingest = Instant::now();

    // Ingest manually via CasStore to verify core performance without CLI overhead
    // (Simulates what `velo ingest` does internally)
    let mut ingested_bytes = 0;
    for entry in walkdir::WalkDir::new(&src_dir) {
        let entry = entry.unwrap();
        if entry.file_type().is_file() {
            let _ = cas.store_file(entry.path()).unwrap();
            ingested_bytes += entry.metadata().unwrap().len();
        }
    }

    let duration = start_ingest.elapsed();
    println!("Ingestion took: {:?}", duration);
    println!(
        "Throughput: {:.2} files/sec",
        FILE_COUNT as f64 / duration.as_secs_f64()
    );
    println!(
        "Throughput: {:.2} MB/sec",
        (ingested_bytes as f64 / 1024.0 / 1024.0) / duration.as_secs_f64()
    );

    // Validation
    let stats = cas.stats().unwrap();
    println!("CAS Stats: {:?}", stats);

    // 50 unique files + 1 shared file = 51 blobs
    assert_eq!(stats.blob_count, 51);
}
