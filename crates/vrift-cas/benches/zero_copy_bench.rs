//! Benchmark: Zero-Copy vs Streaming Pipeline
//!
//! Compare:
//! - zero_copy_ingest (hard_link/rename)
//! - streaming_pipeline (read + write)

use criterion::{black_box, criterion_group, criterion_main, Criterion, BenchmarkId};
use std::fs::{self, File};
use std::io::Write;
use std::path::PathBuf;
use tempfile::TempDir;

use vrift_cas::zero_copy_ingest::{ingest_solid_tier2, ingest_phantom};

fn create_test_files(dir: &std::path::Path, count: usize, size: usize) -> Vec<PathBuf> {
    let content = vec![b'X'; size];
    (0..count)
        .map(|i| {
            let path = dir.join(format!("file_{}.dat", i));
            let mut f = File::create(&path).unwrap();
            // Make each file unique to avoid dedup
            f.write_all(&content).unwrap();
            f.write_all(format!("{}", i).as_bytes()).unwrap();
            path
        })
        .collect()
}

fn bench_zero_copy_solid(c: &mut Criterion) {
    let mut group = c.benchmark_group("ingest_solid_tier2");
    
    for file_count in [10, 100, 1000].iter() {
        let source_dir = TempDir::new().unwrap();
        let cas_dir = TempDir::new().unwrap();
        
        group.bench_with_input(
            BenchmarkId::from_parameter(file_count),
            file_count,
            |b, &count| {
                b.iter_batched(
                    || {
                        // Setup: create fresh files
                        let _ = fs::remove_dir_all(source_dir.path());
                        let _ = fs::remove_dir_all(cas_dir.path());
                        fs::create_dir_all(source_dir.path()).unwrap();
                        fs::create_dir_all(cas_dir.path()).unwrap();
                        create_test_files(source_dir.path(), count, 4096)
                    },
                    |files| {
                        // Benchmark: ingest all files
                        for path in files {
                            let _ = ingest_solid_tier2(black_box(&path), cas_dir.path());
                        }
                    },
                    criterion::BatchSize::SmallInput,
                );
            },
        );
    }
    
    group.finish();
}

fn bench_zero_copy_phantom(c: &mut Criterion) {
    let mut group = c.benchmark_group("ingest_phantom");
    
    for file_count in [10, 100, 1000].iter() {
        let source_dir = TempDir::new().unwrap();
        let cas_dir = TempDir::new().unwrap();
        
        group.bench_with_input(
            BenchmarkId::from_parameter(file_count),
            file_count,
            |b, &count| {
                b.iter_batched(
                    || {
                        // Setup: create fresh files
                        let _ = fs::remove_dir_all(source_dir.path());
                        let _ = fs::remove_dir_all(cas_dir.path());
                        fs::create_dir_all(source_dir.path()).unwrap();
                        fs::create_dir_all(cas_dir.path()).unwrap();
                        create_test_files(source_dir.path(), count, 4096)
                    },
                    |files| {
                        // Benchmark: ingest all files
                        for path in files {
                            let _ = ingest_phantom(black_box(&path), cas_dir.path());
                        }
                    },
                    criterion::BatchSize::SmallInput,
                );
            },
        );
    }
    
    group.finish();
}

criterion_group!(benches, bench_zero_copy_solid, bench_zero_copy_phantom);
criterion_main!(benches);
