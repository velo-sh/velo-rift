//! Streaming Ingest with Producer-Consumer Pipeline
//!
//! Uses crossbeam-channel bounded for efficient producer-consumer:
//! - Scanner thread sends paths via channel (auto backpressure)
//! - Worker threads receive and process (efficient parking)
//!
//! Zero-copy: uses DirEntry::into_path() to transfer PathBuf ownership.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use crossbeam::channel::{self, Receiver, Sender};
use walkdir::WalkDir;

use crate::{CasError, IngestMode, IngestResult};

/// Channel capacity (bounded ring buffer)
const CHANNEL_CAP: usize = 1024;

/// Streaming ingest with producer-consumer pipeline
pub fn streaming_ingest(
    source: &Path,
    cas_root: &Path,
    mode: IngestMode,
    threads: Option<usize>,
) -> Vec<Result<IngestResult, CasError>> {
    use crate::zero_copy_ingest::{ingest_phantom, ingest_solid_tier1, ingest_solid_tier2};

    tracing::info!(
        "[INGEST] streaming_ingest starting: source={:?}, cas={:?}",
        source,
        cas_root
    );

    let (tx, rx): (Sender<PathBuf>, Receiver<PathBuf>) = channel::bounded(CHANNEL_CAP);

    let results: Arc<std::sync::Mutex<Vec<Result<IngestResult, CasError>>>> =
        Arc::new(std::sync::Mutex::new(Vec::new()));

    let num_threads = threads.unwrap_or_else(|| std::cmp::min(4, num_cpus::get() / 2).max(1));
    tracing::info!("[INGEST] Using {} worker threads", num_threads);

    // Scanner thread - sends paths, then drops tx to signal completion
    let source_path = source.to_path_buf();
    tracing::info!("[INGEST] Starting scanner thread for: {:?}", source_path);
    let scanner = std::thread::spawn(move || {
        let mut file_count = 0;
        for entry in WalkDir::new(&source_path)
            .into_iter()
            // Skip .vrift and .git directories entirely (avoids flock deadlock on LMDB lock files)
            .filter_entry(|e| {
                let name = e.file_name().to_str().unwrap_or("");
                name != ".vrift" && name != ".git"
            })
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().is_file())
        {
            let path = entry.into_path();
            file_count += 1;
            if tx.send(path).is_err() {
                tracing::warn!("[INGEST] Scanner: receivers dropped, stopping");
                break;
            }
        }
        tracing::info!("[INGEST] Scanner complete: {} files found", file_count);
    });

    // Spawn worker threads using std::thread (more predictable than rayon scope)
    let cas = cas_root.to_path_buf();
    tracing::info!("[INGEST] Starting worker threads");

    let workers: Vec<_> = (0..num_threads)
        .map(|i| {
            let rx = rx.clone();
            let r = Arc::clone(&results);
            let cas = cas.clone();
            std::thread::spawn(move || {
                tracing::info!("[INGEST] Worker {} started", i);
                let mut processed = 0;
                for path in rx {
                    tracing::info!("[INGEST] Worker {} processing: {:?}", i, path);
                    let result = match mode {
                        IngestMode::Phantom => ingest_phantom(&path, &cas),
                        IngestMode::SolidTier1 => ingest_solid_tier1(&path, &cas),
                        IngestMode::SolidTier2 => ingest_solid_tier2(&path, &cas),
                    };
                    tracing::info!("[INGEST] Worker {} done: {:?}", i, path);
                    r.lock().unwrap().push(result);
                    processed += 1;
                }
                tracing::info!(
                    "[INGEST] Worker {} finished, processed {} files",
                    i,
                    processed
                );
            })
        })
        .collect();

    // Drop original rx so channel disconnects when scanner finishes
    drop(rx);

    // Wait for scanner to complete first
    scanner.join().expect("Scanner thread panicked");
    tracing::info!("[INGEST] Scanner thread joined");

    // Wait for all workers to complete
    for (i, worker) in workers.into_iter().enumerate() {
        worker
            .join()
            .unwrap_or_else(|_| panic!("Worker {} panicked", i));
    }
    tracing::info!("[INGEST] All workers finished");

    Arc::try_unwrap(results)
        .expect("Results still referenced")
        .into_inner()
        .unwrap()
}

/// Streaming ingest with progress callback
pub fn streaming_ingest_with_progress<F>(
    source: &Path,
    cas_root: &Path,
    mode: IngestMode,
    threads: Option<usize>,
    on_progress: F,
) -> Vec<Result<IngestResult, CasError>>
where
    F: Fn(&Result<IngestResult, CasError>, usize) + Send + Sync,
{
    use crate::zero_copy_ingest::{ingest_phantom, ingest_solid_tier1, ingest_solid_tier2};
    use std::sync::atomic::{AtomicUsize, Ordering};

    let (tx, rx): (Sender<PathBuf>, Receiver<PathBuf>) = channel::bounded(CHANNEL_CAP);

    let results: Arc<std::sync::Mutex<Vec<Result<IngestResult, CasError>>>> =
        Arc::new(std::sync::Mutex::new(Vec::new()));
    let counter = Arc::new(AtomicUsize::new(0));
    let on_progress = Arc::new(on_progress);

    let num_threads = threads.unwrap_or_else(|| std::cmp::min(4, num_cpus::get() / 2).max(1));
    let thread_pool = rayon::ThreadPoolBuilder::new()
        .num_threads(num_threads)
        .build()
        .expect("Failed to create thread pool");

    // Scanner
    let source_path = source.to_path_buf();
    let scanner = std::thread::spawn(move || {
        for entry in WalkDir::new(&source_path)
            .into_iter()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().is_file())
        {
            let path = entry.into_path();
            if tx.send(path).is_err() {
                break;
            }
        }
    });

    // Workers
    let cas = cas_root;
    thread_pool.install(|| {
        rayon::scope(|s| {
            for _ in 0..num_threads {
                let rx = rx.clone();
                let r = Arc::clone(&results);
                let cnt = Arc::clone(&counter);
                let cb = Arc::clone(&on_progress);
                s.spawn(move |_| {
                    for path in rx {
                        let result = match mode {
                            IngestMode::Phantom => ingest_phantom(&path, cas),
                            IngestMode::SolidTier1 => ingest_solid_tier1(&path, cas),
                            IngestMode::SolidTier2 => ingest_solid_tier2(&path, cas),
                        };

                        let idx = cnt.fetch_add(1, Ordering::Relaxed);
                        cb(&result, idx);

                        r.lock().unwrap().push(result);
                    }
                });
            }
        });
    });

    scanner.join().expect("Scanner thread panicked");

    Arc::try_unwrap(results)
        .expect("Results still referenced")
        .into_inner()
        .unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn test_streaming_ingest_basic() {
        let temp = tempdir().unwrap();
        let source = temp.path().join("source");
        let cas = temp.path().join("cas");
        fs::create_dir_all(&source).unwrap();
        fs::create_dir_all(&cas).unwrap();

        // Create test files
        for i in 0..100 {
            fs::write(
                source.join(format!("file_{}.txt", i)),
                format!("content {}", i),
            )
            .unwrap();
        }

        let results = streaming_ingest(&source, &cas, IngestMode::SolidTier2, Some(4));

        assert_eq!(results.len(), 100);
        assert!(results.iter().all(|r| r.is_ok()));
    }
}
