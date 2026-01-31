//! Benchmark: Zero-Copy vs Streaming Pipeline
//!
//! Compare:
//! - zero_copy_ingest (hard_link/rename)
//! - Sequential vs Parallel

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion};
use rayon::prelude::*;
use std::fs::{self, File};
use std::io::Write;
use std::path::PathBuf;
use tempfile::TempDir;

use vrift_cas::zero_copy_ingest::{ingest_phantom, ingest_solid_tier2};

fn create_test_files(dir: &std::path::Path, count: usize, size: usize) -> Vec<PathBuf> {
    let content = vec![b'X'; size];
    (0..count)
        .map(|i| {
            let path = dir.join(format!("file_{}.dat", i));
            let mut f = File::create(&path).unwrap();
            f.write_all(&content).unwrap();
            f.write_all(format!("{}", i).as_bytes()).unwrap();
            path
        })
        .collect()
}

// Sequential benchmarks
fn bench_sequential_phantom(c: &mut Criterion) {
    let mut group = c.benchmark_group("phantom_sequential");

    for file_count in [100, 1000].iter() {
        let source_dir = TempDir::new().unwrap();
        let cas_dir = TempDir::new().unwrap();

        group.bench_with_input(
            BenchmarkId::from_parameter(file_count),
            file_count,
            |b, &count| {
                b.iter_batched(
                    || {
                        let _ = fs::remove_dir_all(source_dir.path());
                        let _ = fs::remove_dir_all(cas_dir.path());
                        fs::create_dir_all(source_dir.path()).unwrap();
                        fs::create_dir_all(cas_dir.path()).unwrap();
                        create_test_files(source_dir.path(), count, 4096)
                    },
                    |files| {
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

// Parallel benchmarks with controlled thread count
fn bench_parallel_phantom(c: &mut Criterion) {
    let mut group = c.benchmark_group("phantom_parallel");

    // Test with 2 and 4 threads
    for threads in [2, 4].iter() {
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(*threads)
            .build()
            .unwrap();

        let source_dir = TempDir::new().unwrap();
        let cas_dir = TempDir::new().unwrap();

        group.bench_with_input(BenchmarkId::new("threads", threads), threads, |b, _| {
            b.iter_batched(
                || {
                    let _ = fs::remove_dir_all(source_dir.path());
                    let _ = fs::remove_dir_all(cas_dir.path());
                    fs::create_dir_all(source_dir.path()).unwrap();
                    fs::create_dir_all(cas_dir.path()).unwrap();
                    create_test_files(source_dir.path(), 1000, 4096)
                },
                |files| {
                    let cas_root = cas_dir.path();
                    pool.install(|| {
                        files.par_iter().for_each(|path| {
                            let _ = ingest_phantom(black_box(path), cas_root);
                        });
                    });
                },
                criterion::BatchSize::SmallInput,
            );
        });
    }

    group.finish();
}

fn bench_parallel_solid(c: &mut Criterion) {
    let mut group = c.benchmark_group("solid_parallel");

    for file_count in [100, 1000].iter() {
        let source_dir = TempDir::new().unwrap();
        let cas_dir = TempDir::new().unwrap();

        group.bench_with_input(
            BenchmarkId::from_parameter(file_count),
            file_count,
            |b, &count| {
                b.iter_batched(
                    || {
                        let _ = fs::remove_dir_all(source_dir.path());
                        let _ = fs::remove_dir_all(cas_dir.path());
                        fs::create_dir_all(source_dir.path()).unwrap();
                        fs::create_dir_all(cas_dir.path()).unwrap();
                        create_test_files(source_dir.path(), count, 4096)
                    },
                    |files| {
                        let cas_root = cas_dir.path();
                        files.par_iter().for_each(|path| {
                            let _ = ingest_solid_tier2(black_box(path), cas_root);
                        });
                    },
                    criterion::BatchSize::SmallInput,
                );
            },
        );
    }

    group.finish();
}

criterion_group!(
    benches,
    bench_sequential_phantom,
    bench_parallel_phantom,
    bench_parallel_solid
);
criterion_main!(benches);
