//! Streaming Ingest with Producer-Consumer Pipeline
//!
//! Uses a ring buffer queue to overlap scanning and ingesting:
//! - Scanner thread produces paths
//! - Worker threads consume and process in parallel
//!
//! Zero-copy: uses DirEntry::into_path() to transfer PathBuf ownership.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use crossbeam::queue::ArrayQueue;
use rayon::prelude::*;
use walkdir::WalkDir;

use crate::{CasError, IngestMode, IngestResult};

/// Queue size (number of items in flight)
const QUEUE_SIZE: usize = 1024;

/// Batch size for Rayon processing
const BATCH_SIZE: usize = 256;

/// Item in the ingest queue - uses PathBuf directly (zero-copy from DirEntry)
struct IngestItem {
    /// Path from DirEntry::into_path() - no copy needed
    path: Option<PathBuf>,
    /// File size from dirent (avoid redundant stat)
    file_size: u64,
}

impl IngestItem {
    fn new() -> Self {
        Self {
            path: None,
            file_size: 0,
        }
    }

    /// Take ownership of path from DirEntry (zero-copy)
    fn set(&mut self, path: PathBuf, size: u64) {
        self.path = Some(path);
        self.file_size = size;
    }

    fn path(&self) -> &Path {
        self.path.as_deref().unwrap_or(Path::new(""))
    }

    fn recycle(&mut self) {
        // Just drop the path, no shrink needed
        self.path = None;
        self.file_size = 0;
    }
}

impl Default for IngestItem {
    fn default() -> Self {
        Self::new()
    }
}

/// Pool of reusable IngestItems
struct ItemPool {
    items: ArrayQueue<IngestItem>,
}

impl ItemPool {
    fn new(size: usize) -> Self {
        let items = ArrayQueue::new(size);
        for _ in 0..size {
            let _ = items.push(IngestItem::new());
        }
        Self { items }
    }

    fn acquire(&self) -> IngestItem {
        self.items.pop().unwrap_or_default()
    }

    fn release(&self, mut item: IngestItem) {
        item.recycle();
        let _ = self.items.push(item); // Ignore if pool is full
    }
}

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

    // Work queue: scanner -> workers
    let work_queue: Arc<ArrayQueue<IngestItem>> = Arc::new(ArrayQueue::new(QUEUE_SIZE));

    // Item pool for recycling
    let pool: Arc<ItemPool> = Arc::new(ItemPool::new(QUEUE_SIZE));

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
    let wq: Arc<ArrayQueue<IngestItem>> = Arc::clone(&work_queue);
    let p: Arc<ItemPool> = Arc::clone(&pool);
    let done = Arc::clone(&scanner_done);

    let scanner = std::thread::spawn(move || {
        for entry in WalkDir::new(&source_path)
            .into_iter()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().is_file())
        {
            let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
            let path = entry.into_path(); // zero-copy: take ownership

            let mut item = p.acquire();
            item.set(path, size);

            // Push with backpressure (spin if queue full)
            loop {
                match wq.push(item) {
                    Ok(()) => break,
                    Err(returned) => {
                        item = returned;
                        std::hint::spin_loop();
                    }
                }
            }
        }
        done.store(true, Ordering::Release);
    });

    // Worker loop: consume batches and process with Rayon
    let cas = cas_root.to_path_buf();

    thread_pool.install(|| {
        loop {
            // Collect a batch
            let mut batch: Vec<IngestItem> = Vec::with_capacity(BATCH_SIZE);

            while batch.len() < BATCH_SIZE {
                if let Some(item) = work_queue.pop() {
                    batch.push(item);
                } else if scanner_done.load(Ordering::Acquire) && work_queue.is_empty() {
                    break;
                } else {
                    // Brief pause before retry
                    std::thread::yield_now();
                    break;
                }
            }

            if batch.is_empty() {
                if scanner_done.load(Ordering::Acquire) {
                    break;
                }
                std::thread::yield_now();
                continue;
            }

            // Process batch in parallel
            let batch_results: Vec<_> = batch
                .par_iter()
                .map(|item| {
                    let path = item.path();
                    match mode {
                        IngestMode::Phantom => ingest_phantom(path, &cas),
                        IngestMode::SolidTier1 => ingest_solid_tier1(path, &cas),
                        IngestMode::SolidTier2 => ingest_solid_tier2(path, &cas),
                    }
                })
                .collect();

            // Store results
            results.lock().unwrap().extend(batch_results);

            // Recycle items
            for item in batch {
                pool.release(item);
            }
        }
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

    let work_queue: Arc<ArrayQueue<IngestItem>> = Arc::new(ArrayQueue::new(QUEUE_SIZE));
    let pool: Arc<ItemPool> = Arc::new(ItemPool::new(QUEUE_SIZE));
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
    let wq: Arc<ArrayQueue<IngestItem>> = Arc::clone(&work_queue);
    let p: Arc<ItemPool> = Arc::clone(&pool);
    let done: Arc<AtomicBool> = Arc::clone(&scanner_done);

    let scanner = std::thread::spawn(move || {
        for entry in WalkDir::new(&source_path)
            .into_iter()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().is_file())
        {
            let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
            let path = entry.into_path(); // zero-copy
            let mut item = p.acquire();
            item.set(path, size);

            loop {
                match wq.push(item) {
                    Ok(()) => break,
                    Err(returned) => {
                        item = returned;
                        std::hint::spin_loop();
                    }
                }
            }
        }
        done.store(true, Ordering::Release);
    });

    // Workers
    let cas = cas_root.to_path_buf();

    thread_pool.install(|| loop {
        let mut batch: Vec<IngestItem> = Vec::with_capacity(BATCH_SIZE);

        while batch.len() < BATCH_SIZE {
            if let Some(item) = work_queue.pop() {
                batch.push(item);
            } else if scanner_done.load(Ordering::Acquire) && work_queue.is_empty() {
                break;
            } else {
                std::thread::yield_now();
                break;
            }
        }

        if batch.is_empty() {
            if scanner_done.load(Ordering::Acquire) {
                break;
            }
            std::thread::yield_now();
            continue;
        }

        let batch_results: Vec<_> = batch
            .par_iter()
            .map(|item| {
                let path = item.path();
                let result = match mode {
                    IngestMode::Phantom => ingest_phantom(path, &cas),
                    IngestMode::SolidTier1 => ingest_solid_tier1(path, &cas),
                    IngestMode::SolidTier2 => ingest_solid_tier2(path, &cas),
                };

                let idx = counter.fetch_add(1, Ordering::Relaxed);
                on_progress(&result, idx);

                result
            })
            .collect();

        results.lock().unwrap().extend(batch_results);

        for item in batch {
            pool.release(item);
        }
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

    #[test]
    fn test_item_set_and_path() {
        let mut item = IngestItem::new();
        assert!(item.path.is_none());

        // Set path
        item.set(PathBuf::from("/short/path"), 100);
        assert_eq!(item.path().to_str().unwrap(), "/short/path");
        assert_eq!(item.file_size, 100);

        // Set very long path
        let long_path = "/".to_string() + &"a".repeat(3000);
        item.set(PathBuf::from(&long_path), 200);
        assert_eq!(item.path().to_str().unwrap(), long_path);

        // Recycle should clear
        item.recycle();
        assert!(item.path.is_none());
    }
}
