//! Parallel Zero-Copy Ingest using Rayon (RFC-0039 Aligned)
//!
//! Combines RFC-0039 zero-copy operations with Rayon parallel iteration
//! for multi-core CPU utilization.
//!
//! # Thread Pool Configuration
//!
//! To preserve system resources (for compilation, etc.), we limit Rayon threads to:
//! - Half of CPU cores, or
//! - Maximum of 4 threads
//!
//! # Lock Safety
//!
//! Each file uses a shared flock that:
//! - Auto-releases on drop (RAII)
//! - Uses non-blocking attempts with retry + exponential backoff
//! - Prevents deadlocks (each file has independent lock)
//!
//! # Performance
//!
//! On a typical 8-core system, expect 2-4x speedup depending on:
//! - File sizes (smaller files = more parallelism benefit)
//! - Storage speed (SSD vs HDD)
//! - I/O saturation point

use std::path::{Path, PathBuf};

use dashmap::DashSet;
use nix::libc;
use rayon::prelude::*;
use rayon::ThreadPool;

use crate::{
    ingest_phantom, ingest_solid_tier1, ingest_solid_tier1_dedup, ingest_solid_tier2,
    ingest_solid_tier2_dedup, IngestResult, Result,
};

// ============================================================================
// Thread Pool Configuration
// ============================================================================

/// Maximum threads for parallel ingest (preserve system resources)
pub const MAX_INGEST_THREADS: usize = 4;

/// Calculate default thread count: min(cpu_cores / 2, MAX_INGEST_THREADS)
pub fn default_thread_count() -> usize {
    (num_cpus::get() / 2).clamp(1, MAX_INGEST_THREADS)
}

/// Create a thread pool with specified thread count
///
/// # Arguments
///
/// * `threads` - Number of threads. None = use default (min(cpu/2, 4))
fn create_thread_pool(threads: Option<usize>) -> ThreadPool {
    let num_threads = threads.unwrap_or_else(default_thread_count);
    rayon::ThreadPoolBuilder::new()
        .num_threads(num_threads)
        .thread_name(|i| format!("vrift-ingest-{}", i))
        .build()
        .expect("Failed to create ingest thread pool")
}

// ============================================================================
// Ingest Mode
// ============================================================================

/// Zero-copy ingest mode selection
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IngestMode {
    /// Tier-1 Immutable: hard_link to CAS + symlink replacement
    /// Safe for rollback (original replaced with symlink to CAS)
    SolidTier1,

    /// Tier-2 Mutable: hard_link only (keep original)
    /// Safe for rollback (original unchanged)
    SolidTier2,

    /// Phantom: rename to CAS (atomic move)
    /// NOT safe for rollback without explicit restore
    Phantom,
}

impl std::fmt::Display for IngestMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            IngestMode::SolidTier1 => write!(f, "Solid Tier-1 (hard_link + symlink)"),
            IngestMode::SolidTier2 => write!(f, "Solid Tier-2 (hard_link, keep original)"),
            IngestMode::Phantom => write!(f, "Phantom (rename → CAS)"),
        }
    }
}

// ============================================================================
// Parallel Ingest
// ============================================================================

/// Result from parallel ingest operation
#[derive(Debug)]
pub struct ParallelIngestStats {
    /// Number of files successfully ingested
    pub success_count: usize,
    /// Number of files that failed
    pub error_count: usize,
    /// Total bytes ingested
    pub total_bytes: u64,
    /// EXDEV fallback count (cross-device)
    pub fallback_count: usize,
}

/// Ingest multiple files in parallel using zero-copy operations
///
/// Uses Rayon's par_iter to process files across multiple CPU cores.
/// Each file is processed using the appropriate zero-copy function
/// based on the selected mode.
///
/// # Arguments
///
/// * `files` - Vector of file paths to ingest
/// * `cas_root` - Root directory of the CAS store
/// * `mode` - Ingest mode (SolidTier1, SolidTier2, or Phantom)
///
/// # Returns
///
/// Vector of results, one per input file, in the same order.
///
/// # Example
///
/// ```ignore
/// use vrift_cas::{parallel_ingest, IngestMode};
///
/// let files = vec![
///     PathBuf::from("/path/to/file1"),
///     PathBuf::from("/path/to/file2"),
/// ];
///
/// let results = parallel_ingest(&files, Path::new("/cas"), IngestMode::SolidTier2);
/// for result in results {
///     match result {
///         Ok(r) => println!("Ingested: {} bytes", r.size),
///         Err(e) => eprintln!("Error: {}", e),
///     }
/// }
/// ```
pub fn parallel_ingest(
    files: &[PathBuf],
    cas_root: &Path,
    mode: IngestMode,
) -> Vec<Result<IngestResult>> {
    parallel_ingest_with_threads(files, cas_root, mode, None)
}

/// Ingest multiple files in parallel with custom thread count
///
/// # Arguments
///
/// * `files` - Vector of file paths to ingest
/// * `cas_root` - Root directory of the CAS store
/// * `mode` - Ingest mode (SolidTier1, SolidTier2, or Phantom)
/// * `threads` - Number of threads (None = use default: min(cpu/2, 4))
pub fn parallel_ingest_with_threads(
    files: &[PathBuf],
    cas_root: &Path,
    mode: IngestMode,
    threads: Option<usize>,
) -> Vec<Result<IngestResult>> {
    let pool = create_thread_pool(threads);

    // In-memory dedup set for tracking seen hashes
    // Used for both SolidTier1 and SolidTier2
    let seen_hashes: DashSet<String> = DashSet::new();

    pool.install(|| {
        files
            .par_iter()
            .map(|path| {
                match mode {
                    // Use dedup-aware versions to skip redundant hard_link calls
                    IngestMode::SolidTier1 => {
                        ingest_solid_tier1_dedup(path, cas_root, &seen_hashes)
                    }
                    IngestMode::SolidTier2 => {
                        ingest_solid_tier2_dedup(path, cas_root, &seen_hashes)
                    }
                    IngestMode::Phantom => ingest_phantom(path, cas_root),
                }
            })
            .collect()
    })
}

/// Ingest files in parallel with real-time progress callback
///
/// Unlike `parallel_ingest_with_threads`, this function calls the progress callback
/// after each file is processed, enabling real-time progress bar updates.
///
/// # Arguments
///
/// * `files` - Vector of file paths to ingest
/// * `cas_root` - Root directory of the CAS store
/// * `mode` - Ingest mode
/// * `threads` - Number of threads
/// * `on_progress` - Callback called after each file: (result, index)
pub fn parallel_ingest_with_progress<F>(
    files: &[PathBuf],
    cas_root: &Path,
    mode: IngestMode,
    threads: Option<usize>,
    on_progress: F,
) -> Vec<Result<IngestResult>>
where
    F: Fn(&Result<IngestResult>, usize) + Send + Sync,
{
    use std::sync::atomic::{AtomicUsize, Ordering};

    let pool = create_thread_pool(threads);
    let seen_hashes: DashSet<String> = DashSet::new();
    let counter = AtomicUsize::new(0);

    pool.install(|| {
        files
            .par_iter()
            .map(|path| {
                let result = match mode {
                    IngestMode::SolidTier1 => {
                        ingest_solid_tier1_dedup(path, cas_root, &seen_hashes)
                    }
                    IngestMode::SolidTier2 => {
                        ingest_solid_tier2_dedup(path, cas_root, &seen_hashes)
                    }
                    IngestMode::Phantom => ingest_phantom(path, cas_root),
                };

                // Call progress callback with current count
                let idx = counter.fetch_add(1, Ordering::Relaxed);
                on_progress(&result, idx);

                result
            })
            .collect()
    })
}

/// Ingest files in parallel with fallback for cross-device errors
///
/// When EXDEV (cross-device link) error occurs, falls back to
/// traditional copy-based ingestion.
///
/// # Arguments
///
/// * `files` - Vector of (path, size) tuples
/// * `cas_root` - Root directory of the CAS store
/// * `mode` - Ingest mode
/// * `cas` - CasStore for fallback operations
///
/// # Returns
///
/// Tuple of (results, stats) where results contains all successful ingests
/// and stats provides summary metrics.
pub fn parallel_ingest_with_fallback(
    files: &[(PathBuf, u64)],
    cas_root: &Path,
    mode: IngestMode,
    cas: &crate::CasStore,
) -> (Vec<IngestResult>, ParallelIngestStats) {
    use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

    let success_count = AtomicUsize::new(0);
    let error_count = AtomicUsize::new(0);
    let total_bytes = AtomicU64::new(0);
    let fallback_count = AtomicUsize::new(0);

    let results: Vec<Option<IngestResult>> = files
        .par_iter()
        .map(|(path, _size)| {
            let result = match mode {
                IngestMode::SolidTier1 => ingest_solid_tier1(path, cas_root),
                IngestMode::SolidTier2 => ingest_solid_tier2(path, cas_root),
                IngestMode::Phantom => ingest_phantom(path, cas_root),
            };

            match result {
                Ok(ingest_result) => {
                    success_count.fetch_add(1, Ordering::Relaxed);
                    total_bytes.fetch_add(ingest_result.size, Ordering::Relaxed);
                    Some(ingest_result)
                }
                Err(crate::CasError::Io(ref io_err))
                    if io_err.raw_os_error() == Some(libc::EXDEV) =>
                {
                    // Cross-device fallback: read + store
                    match std::fs::read(path) {
                        Ok(content) => {
                            let hash = crate::CasStore::compute_hash(&content);
                            let size = content.len() as u64;
                            match cas.store(&content) {
                                Ok(_) => {
                                    success_count.fetch_add(1, Ordering::Relaxed);
                                    total_bytes.fetch_add(size, Ordering::Relaxed);
                                    fallback_count.fetch_add(1, Ordering::Relaxed);
                                    Some(IngestResult {
                                        source_path: path.clone(),
                                        hash,
                                        size,
                                        was_new: true,
                                        skipped_by_cache: false,
                                        mtime: 0, // fallback path: no metadata available
                                        mode: 0o644,
                                    })
                                }
                                Err(_) => {
                                    error_count.fetch_add(1, Ordering::Relaxed);
                                    None
                                }
                            }
                        }
                        Err(_) => {
                            error_count.fetch_add(1, Ordering::Relaxed);
                            None
                        }
                    }
                }
                Err(_) => {
                    error_count.fetch_add(1, Ordering::Relaxed);
                    None
                }
            }
        })
        .collect();

    let successful_results: Vec<IngestResult> = results.into_iter().flatten().collect();

    let stats = ParallelIngestStats {
        success_count: success_count.load(Ordering::Relaxed),
        error_count: error_count.load(Ordering::Relaxed),
        total_bytes: total_bytes.load(Ordering::Relaxed),
        fallback_count: fallback_count.load(Ordering::Relaxed),
    };

    (successful_results, stats)
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn setup() -> (TempDir, TempDir, Vec<PathBuf>) {
        let source_dir = TempDir::new().unwrap();
        let cas_dir = TempDir::new().unwrap();

        // Create test files
        let mut files = Vec::new();
        for i in 0..10 {
            let path = source_dir.path().join(format!("file_{}.txt", i));
            fs::write(&path, format!("Content of file {}", i)).unwrap();
            files.push(path);
        }

        (source_dir, cas_dir, files)
    }

    #[test]
    fn test_parallel_ingest_solid_tier2() {
        let (_source_dir, cas_dir, files) = setup();

        let results = parallel_ingest(&files, cas_dir.path(), IngestMode::SolidTier2);

        // All should succeed
        assert_eq!(results.len(), 10);
        for result in &results {
            assert!(result.is_ok(), "Expected Ok, got {:?}", result);
        }

        // Verify CAS has files
        let cas_blake3 = cas_dir.path().join("blake3");
        assert!(cas_blake3.exists(), "CAS blake3 directory should exist");
    }

    #[test]
    fn test_parallel_ingest_phantom() {
        let (_source_dir, cas_dir, files) = setup();
        let original_files: Vec<_> = files.clone();

        let results = parallel_ingest(&files, cas_dir.path(), IngestMode::Phantom);

        // All should succeed
        assert_eq!(results.len(), 10);
        for result in &results {
            assert!(result.is_ok());
        }

        // Original files should be gone (moved)
        for path in &original_files {
            assert!(!path.exists(), "Phantom mode should move files: {:?}", path);
        }
    }

    #[test]
    fn test_ingest_mode_display() {
        assert_eq!(
            format!("{}", IngestMode::SolidTier1),
            "Solid Tier-1 (hard_link + symlink)"
        );
        assert_eq!(
            format!("{}", IngestMode::SolidTier2),
            "Solid Tier-2 (hard_link, keep original)"
        );
        assert_eq!(format!("{}", IngestMode::Phantom), "Phantom (rename → CAS)");
    }

    #[test]
    fn test_parallel_ingest_stats() {
        let source_dir = TempDir::new().unwrap();
        let cas_dir = TempDir::new().unwrap();

        // Create test files with known sizes
        let mut files = Vec::new();
        for i in 0..5 {
            let path = source_dir.path().join(format!("file_{}.txt", i));
            let content = format!("Content {}", i);
            fs::write(&path, &content).unwrap();
            files.push((path, content.len() as u64));
        }

        let cas = crate::CasStore::new(cas_dir.path()).unwrap();
        let (results, stats) =
            parallel_ingest_with_fallback(&files, cas_dir.path(), IngestMode::SolidTier2, &cas);

        assert_eq!(results.len(), 5);
        assert_eq!(stats.success_count, 5);
        assert_eq!(stats.error_count, 0);
        assert!(stats.total_bytes > 0);
    }
}
