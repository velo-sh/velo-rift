//! Streaming Ingest with Producer-Consumer Pipeline
//!
//! Uses a ring buffer queue to overlap scanning and ingesting:
//! - Scanner thread produces paths to bounded queue
//! - Worker threads pop and process independently (no batch blocking)
//!
//! Zero-copy: uses DirEntry::into_path() to transfer PathBuf ownership.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use crossbeam::queue::ArrayQueue;
use walkdir::WalkDir;

use crate::{CasError, IngestMode, IngestResult};

/// Queue size (number of paths in flight)
const QUEUE_SIZE: usize = 1024;

/// Streaming ingest with producer-consumer pipeline
///
/// # Arguments
/// * `source` - Directory to scan
/// * `cas_root` - CAS storage root
/// * `mode` - Ingest mode (Solid/Phantom)
/// * `threads` - Number of worker threads (None = auto)
///
/// # Returns
/// Vector of ingest results
pub fn streaming_ingest(
    source: &Path,
    cas_root: &Path,
    mode: IngestMode,
    threads: Option<usize>,
) -> Vec<Result<IngestResult, CasError>> {
    use crate::zero_copy_ingest::{ingest_phantom, ingest_solid_tier1, ingest_solid_tier2};

    // Work queue: scanner -> workers (direct PathBuf, no wrapper)
    let work_queue: Arc<ArrayQueue<PathBuf>> = Arc::new(ArrayQueue::new(QUEUE_SIZE));

    // Scanner done flag
    let scanner_done = Arc::new(AtomicBool::new(false));

    // Results collector
    let results: Arc<std::sync::Mutex<Vec<Result<IngestResult, CasError>>>> =
        Arc::new(std::sync::Mutex::new(Vec::new()));

    // Configure Rayon thread pool
    let num_threads = threads.unwrap_or_else(num_cpus::get);
    let thread_pool = rayon::ThreadPoolBuilder::new()
        .num_threads(num_threads)
        .build()
        .expect("Failed to create thread pool");

    // Scanner thread
    let source_path = source.to_path_buf();
    let scanner_wq = Arc::clone(&work_queue);
    let scanner_done_flag = Arc::clone(&scanner_done);

    let scanner = std::thread::spawn(move || {
        for entry in WalkDir::new(&source_path)
            .into_iter()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().is_file())
        {
            let mut path = entry.into_path();
            loop {
                match scanner_wq.push(path) {
                    Ok(()) => break,
                    Err(returned) => {
                        path = returned;
                        std::hint::spin_loop();
                    }
                }
            }
        }
        scanner_done_flag.store(true, Ordering::Release);
    });

    // Worker loop
    let cas = cas_root;

    // Independent workers: each pops and processes directly
    thread_pool.install(|| {
        rayon::scope(|s| {
            for _ in 0..num_threads {
                s.spawn(|_| loop {
                    if let Some(path) = work_queue.pop() {
                        let result = match mode {
                            IngestMode::Phantom => ingest_phantom(&path, cas),
                            IngestMode::SolidTier1 => ingest_solid_tier1(&path, cas),
                            IngestMode::SolidTier2 => ingest_solid_tier2(&path, cas),
                        };
                        results.lock().unwrap().push(result);
                    } else if scanner_done.load(Ordering::Acquire) && work_queue.is_empty() {
                        break;
                    } else {
                        std::thread::yield_now();
                    }
                });
            }
        });
    });

    // Wait for scanner
    scanner.join().expect("Scanner thread panicked");

    // Return results
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
    use std::sync::atomic::AtomicUsize;

    let work_queue: Arc<ArrayQueue<PathBuf>> = Arc::new(ArrayQueue::new(QUEUE_SIZE));
    let scanner_done = Arc::new(AtomicBool::new(false));
    let results: Arc<std::sync::Mutex<Vec<Result<IngestResult, CasError>>>> =
        Arc::new(std::sync::Mutex::new(Vec::new()));
    let counter = Arc::new(AtomicUsize::new(0));
    let on_progress = Arc::new(on_progress);

    let num_threads = threads.unwrap_or_else(num_cpus::get);
    let thread_pool = rayon::ThreadPoolBuilder::new()
        .num_threads(num_threads)
        .build()
        .expect("Failed to create thread pool");

    // Scanner
    let source_path = source.to_path_buf();
    let scanner_wq = Arc::clone(&work_queue);
    let scanner_done_flag = Arc::clone(&scanner_done);

    let scanner = std::thread::spawn(move || {
        for entry in WalkDir::new(&source_path)
            .into_iter()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().is_file())
        {
            let mut path = entry.into_path();
            loop {
                match scanner_wq.push(path) {
                    Ok(()) => break,
                    Err(returned) => {
                        path = returned;
                        std::hint::spin_loop();
                    }
                }
            }
        }
        scanner_done_flag.store(true, Ordering::Release);
    });

    // Workers
    let cas = cas_root;

    thread_pool.install(|| {
        rayon::scope(|s| {
            for _ in 0..num_threads {
                s.spawn(|_| loop {
                    if let Some(path) = work_queue.pop() {
                        let result = match mode {
                            IngestMode::Phantom => ingest_phantom(&path, cas),
                            IngestMode::SolidTier1 => ingest_solid_tier1(&path, cas),
                            IngestMode::SolidTier2 => ingest_solid_tier2(&path, cas),
                        };

                        let idx = counter.fetch_add(1, Ordering::Relaxed);
                        on_progress(&result, idx);

                        results.lock().unwrap().push(result);
                    } else if scanner_done.load(Ordering::Acquire) && work_queue.is_empty() {
                        break;
                    } else {
                        std::thread::yield_now();
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
