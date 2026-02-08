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
use jwalk::WalkDir;

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

    let num_threads = threads.unwrap_or_else(|| std::cmp::min(4, num_cpus::get() / 2).max(1));
    tracing::info!("[INGEST] Using {} worker threads", num_threads);

    // Scanner thread - sends paths, then drops tx to signal completion
    let source_path = source.to_path_buf();
    tracing::info!("[INGEST] Starting scanner thread for: {:?}", source_path);
    let scanner = std::thread::spawn(move || {
        let mut file_count = 0;
        for entry in WalkDir::new(&source_path)
            .process_read_dir(|_depth, _path, _state, children| {
                children.retain(|entry| {
                    entry.as_ref().map_or(true, |e| {
                        let name = e.file_name.to_str().unwrap_or("");
                        name != ".vrift" && name != ".git"
                    })
                });
            })
            .into_iter()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().is_file())
        {
            let path = entry.path();
            file_count += 1;
            if tx.send(path).is_err() {
                tracing::warn!("[INGEST] Scanner: receivers dropped, stopping");
                break;
            }
        }
        tracing::info!("[INGEST] Scanner complete: {} files found", file_count);
    });

    // Phase4-#3: Per-worker local Vec (no Mutex contention)
    let cas = cas_root.to_path_buf();
    tracing::info!("[INGEST] Starting worker threads");

    let workers: Vec<_> = (0..num_threads)
        .map(|i| {
            let rx = rx.clone();
            let cas = cas.clone();
            std::thread::spawn(move || -> Vec<Result<IngestResult, CasError>> {
                let mut local_results = Vec::new();
                let mut processed = 0;
                for path in rx {
                    tracing::trace!("[INGEST] Worker {} processing: {:?}", i, path);
                    let result = match mode {
                        IngestMode::Phantom => ingest_phantom(&path, &cas),
                        IngestMode::SolidTier1 => ingest_solid_tier1(&path, &cas),
                        IngestMode::SolidTier2 => ingest_solid_tier2(&path, &cas),
                    };
                    tracing::trace!("[INGEST] Worker {} done: {:?}", i, path);
                    local_results.push(result);
                    processed += 1;
                }
                tracing::info!(
                    "[INGEST] Worker {} finished, processed {} files",
                    i,
                    processed
                );
                local_results
            })
        })
        .collect();

    // Drop original rx so channel disconnects when scanner finishes
    drop(rx);

    // Wait for scanner to complete first
    scanner.join().expect("Scanner thread panicked");
    tracing::info!("[INGEST] Scanner thread joined");

    // Collect per-worker results into a single Vec (no lock, just extend)
    let mut all_results = Vec::new();
    for (i, worker) in workers.into_iter().enumerate() {
        let worker_results = worker
            .join()
            .unwrap_or_else(|_| panic!("Worker {} panicked", i));
        all_results.extend(worker_results);
    }
    tracing::info!("[INGEST] All workers finished");

    all_results
}

/// Streaming ingest with mtime+size cache skip (P0 Optimization)
///
/// Same producer-consumer pipeline as `streaming_ingest`, but workers check
/// (mtime, size) against a cache before hashing. On cache hit, returns the
/// cached content_hash without reading the file.
///
/// # Arguments
///
/// * `source` - Source directory to scan
/// * `cas_root` - CAS storage root
/// * `mode` - Ingest mode
/// * `threads` - Worker thread count
/// * `cache_lookup` - Closure: manifest_key → Option<CacheHint>
pub fn streaming_ingest_cached<F>(
    source: &Path,
    cas_root: &Path,
    mode: IngestMode,
    threads: Option<usize>,
    cache_lookup: F,
) -> Vec<Result<IngestResult, CasError>>
where
    F: Fn(&str) -> Option<crate::zero_copy_ingest::CacheHint> + Send + Sync + 'static,
{
    use crate::zero_copy_ingest::{ingest_phantom, ingest_solid_tier1, ingest_solid_tier2_cached};

    tracing::info!(
        "[INGEST] streaming_ingest_cached starting: source={:?}, cas={:?}",
        source,
        cas_root
    );

    let (tx, rx): (Sender<PathBuf>, Receiver<PathBuf>) = channel::bounded(CHANNEL_CAP);

    let num_threads = threads.unwrap_or_else(|| std::cmp::min(4, num_cpus::get() / 2).max(1));
    tracing::info!(
        "[INGEST] Using {} worker threads (cached mode)",
        num_threads
    );

    // Scanner thread
    let source_path = source.to_path_buf();
    let scanner_source = source_path.clone();
    let scanner = std::thread::spawn(move || {
        let mut file_count = 0;
        for entry in WalkDir::new(&scanner_source)
            .process_read_dir(|_depth, _path, _state, children| {
                children.retain(|entry| {
                    entry.as_ref().map_or(true, |e| {
                        let name = e.file_name.to_str().unwrap_or("");
                        name != ".vrift" && name != ".git"
                    })
                });
            })
            .into_iter()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().is_file())
        {
            let path = entry.path();
            file_count += 1;
            if tx.send(path).is_err() {
                break;
            }
        }
        tracing::info!("[INGEST] Scanner complete: {} files found", file_count);
    });

    // Phase4-#3: Per-worker local Vec (no Mutex contention)
    let cas = cas_root.to_path_buf();
    let cache_lookup = Arc::new(cache_lookup);

    let workers: Vec<_> = (0..num_threads)
        .map(|i| {
            let rx = rx.clone();
            let cas = cas.clone();
            let source_root = source_path.clone();
            let cache = Arc::clone(&cache_lookup);
            std::thread::spawn(move || -> Vec<Result<IngestResult, CasError>> {
                let mut local_results = Vec::new();
                let mut processed = 0u64;
                let mut cache_hits = 0u64;
                for path in rx {
                    let result = match mode {
                        IngestMode::SolidTier2 => {
                            // Compute manifest key: /relative/path from source root
                            let manifest_key = match path.strip_prefix(&source_root) {
                                Ok(rel) => format!("/{}", rel.display()),
                                Err(_) => format!(
                                    "/{}",
                                    path.file_name().unwrap_or_default().to_string_lossy()
                                ),
                            };
                            let res =
                                ingest_solid_tier2_cached(&path, &cas, &manifest_key, &*cache);
                            if let Ok(ref r) = res {
                                if r.skipped_by_cache {
                                    cache_hits += 1;
                                }
                            }
                            res
                        }
                        IngestMode::Phantom => ingest_phantom(&path, &cas),
                        IngestMode::SolidTier1 => ingest_solid_tier1(&path, &cas),
                    };
                    local_results.push(result);
                    processed += 1;
                }
                tracing::info!(
                    "[INGEST] Worker {} finished: processed={}, cache_hits={}",
                    i,
                    processed,
                    cache_hits
                );
                local_results
            })
        })
        .collect();

    drop(rx);
    scanner.join().expect("Scanner thread panicked");

    // Collect per-worker results (no lock, just extend)
    let mut all_results = Vec::new();
    for (i, worker) in workers.into_iter().enumerate() {
        let worker_results = worker
            .join()
            .unwrap_or_else(|_| panic!("Worker {} panicked", i));
        all_results.extend(worker_results);
    }

    all_results
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
            let path = entry.path();
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
