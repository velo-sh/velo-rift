use rayon::prelude::*;
use std::fs::{self, File};
use std::io::Write;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Instant;
use tempfile::TempDir;
use vrift_cas::CasStore;

fn main() {
    const FILE_COUNT: usize = 10_000;

    // Keep tempdir alive for the entire test
    let temp = TempDir::new().unwrap();
    let src_dir = temp.path().join("src");
    let cas_dir_serial = temp.path().join("cas_serial");
    let cas_dir_parallel = temp.path().join("cas_parallel");
    fs::create_dir_all(&src_dir).unwrap();
    fs::create_dir_all(&cas_dir_serial).unwrap();
    fs::create_dir_all(&cas_dir_parallel).unwrap();

    println!("=== Parallel vs Serial Ingest Benchmark ===\n");
    println!("Generating {} files...", FILE_COUNT);
    let start_gen = Instant::now();

    // Parallel file generation
    (0..FILE_COUNT).into_par_iter().for_each(|i| {
        let p = src_dir.join(format!("file_{}.txt", i));
        let mut f = File::create(&p).unwrap();
        let content = if i % 2 == 0 {
            format!("content unique {}", i)
        } else {
            "shared content".to_string()
        };
        writeln!(f, "{}", content).unwrap();
    });

    println!("Generation took: {:?}\n", start_gen.elapsed());

    // Collect file paths AFTER generation
    let files: Vec<_> = walkdir::WalkDir::new(&src_dir)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
        .map(|e| e.path().to_path_buf())
        .collect();

    println!("Collected {} files\n", files.len());

    // === Serial Test ===
    println!("--- Serial Ingest ---");
    let cas_serial = CasStore::new(&cas_dir_serial).unwrap();
    let start = Instant::now();
    for path in &files {
        let _ = cas_serial.store_file(path).unwrap();
    }
    let serial_time = start.elapsed();
    let serial_stats = cas_serial.stats().unwrap();
    println!("Time: {:?}", serial_time);
    println!(
        "Throughput: {:.2} files/sec",
        FILE_COUNT as f64 / serial_time.as_secs_f64()
    );
    println!("Blobs: {}\n", serial_stats.blob_count);

    // === Parallel Test ===
    println!("--- Parallel Ingest (Rayon) ---");
    let cas_parallel = Arc::new(CasStore::new(&cas_dir_parallel).unwrap());
    let counter = Arc::new(AtomicUsize::new(0));
    let start = Instant::now();

    files.par_iter().for_each(|path| {
        if let Err(e) = cas_parallel.store_file(path) {
            eprintln!("Error storing {:?}: {}", path, e);
        }
        counter.fetch_add(1, Ordering::Relaxed);
    });

    let parallel_time = start.elapsed();
    let parallel_stats = cas_parallel.stats().unwrap();
    println!("Time: {:?}", parallel_time);
    println!(
        "Throughput: {:.2} files/sec",
        FILE_COUNT as f64 / parallel_time.as_secs_f64()
    );
    println!("Blobs: {}\n", parallel_stats.blob_count);

    // === Summary ===
    let speedup = serial_time.as_secs_f64() / parallel_time.as_secs_f64();
    println!("=== Summary ===");
    println!(
        "Serial:   {:?} ({:.0} files/sec)",
        serial_time,
        FILE_COUNT as f64 / serial_time.as_secs_f64()
    );
    println!(
        "Parallel: {:?} ({:.0} files/sec)",
        parallel_time,
        FILE_COUNT as f64 / parallel_time.as_secs_f64()
    );
    println!("Speedup:  {:.2}x", speedup);

    // Keep temp alive until here
    drop(temp);
}
